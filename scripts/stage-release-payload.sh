#!/usr/bin/env bash
# Stage the release payload for one platform (issue #288).
#
# The rebuild-and-compare job has to stage *exactly* what the release build
# stages, or it compares two different things and its verdict is noise. The
# staging used to live inline in `build-package.yml` (three per-OS steps), where
# a second copy in `reproducible-build.yml` could silently drift â€” the classic
# failure mode that makes a green reproducibility gate meaningless.
#
# One script, two callers, one layout:
#
#   Linux/macOS   staging/rivulet-gui              (stripped)
#                 staging/rivulet-<os>-<arch>-gui  (platform-qualified copy)
#   Windows       staging/rivulet-gui.exe
#                 staging/rivulet.exe              (launcher)
#                 staging/rivulet-updater.exe
#
# The platform-qualified copy exists because Linux and macOS both produce a
# file called `rivulet-gui`, and the release job downloads every platform's
# artifact with `merge-multiple: true`: identical names collapse into one
# asset and one platform's binary silently disappears (observed on alpha.138,
# where SHA256SUMS listed `rivulet-gui` exactly once).
#
# Usage:
#   scripts/stage-release-payload.sh --target x86_64-unknown-linux-gnu
#   scripts/stage-release-payload.sh --target x86_64-pc-windows-msvc
#   scripts/stage-release-payload.sh --self-test
#
# Options:
#   --target TRIPLE                  required (else $TARGET)
#   --platform linux|macos|windows   optional; derived from --target, and
#                                    refused when it contradicts the triple
#   --target-dir DIR                 default: $CARGO_TARGET_DIR, else target
#   --staging DIR                    default: staging
#   --strip auto|none|<path>         default: $RIVULET_STRIP, else auto
#
# `--strip auto` requires a working `strip` and fails loudly when it is
# missing, because a silently unstripped payload is a *different* payload than
# the one a previous release shipped. `--strip none` is the explicit opt-out
# (Windows never strips: the MSVC binaries carry a separate PDB).
set -euo pipefail

die() {
    printf 'stage-release-payload: %s\n' "$*" >&2
    exit 1
}

# The triple's first component is the architecture every release asset name
# already uses (`rivulet-linux-x86_64-gui`, `rivulet-macos-aarch64-gui`).
arch_of() {
    case "${1%%-*}" in
        x86_64) printf 'x86_64' ;;
        aarch64) printf 'aarch64' ;;
        *) return 1 ;;
    esac
}

# The platform is derived from the target triple rather than passed in, so the
# release build (which only knows its target) and the rebuild-and-compare job
# (which has a platform in its matrix) cannot disagree about which staging
# layout applies -- disagreeing layouts would compare two different payloads.
platform_of_target() {
    case "$1" in
        *-linux-*) printf 'linux' ;;
        *-apple-darwin) printf 'macos' ;;
        *-windows-*) printf 'windows' ;;
        *) return 1 ;;
    esac
}

strip_binary() {
    local path="$1"
    case "$strip_mode" in
        none) return 0 ;;
        auto) strip "$path" ;;
        *)
            [[ -x "$strip_mode" ]] || die "strip tool is not executable: $strip_mode"
            "$strip_mode" "$path"
            ;;
    esac
}

# Resolve the strip tool *before* touching the staging directory, so a missing
# tool cannot leave a half-staged payload behind or silently ship unstripped
# binaries that differ from every previous release.
require_strip_tool() {
    case "$strip_mode" in
        none) return 0 ;;
        auto)
            command -v strip >/dev/null 2>&1 ||
                die "no strip in PATH; install binutils or pass --strip none explicitly"
            ;;
        *)
            [[ -x "$strip_mode" ]] || die "strip tool is not executable: $strip_mode"
            ;;
    esac
}

copy() {
    local from="$1" to="$2"
    [[ -f "$from" ]] || die "missing build output $from (did the release build finish?)"
    cp "$from" "$to"
}

stage() {
    [[ -n "$target" ]] || die "--target is required (or set \$TARGET)"

    local derived
    derived="$(platform_of_target "$target")" ||
        die "cannot derive the release platform from $target"
    if [[ -n "$platform" && "$platform" != "$derived" ]]; then
        die "--platform $platform contradicts the target $target (which is $derived)"
    fi
    platform="$derived"

    local arch
    arch="$(arch_of "$target")" || die "cannot derive the release architecture from $target"
    local release="$target_dir/$target/release"

    # Every precondition first, every write second: a refused invocation must
    # not leave a staging directory that looks like a payload to the
    # fingerprint step.
    if [[ "$platform" != "windows" ]]; then
        require_strip_tool
    fi

    local gui="$release/rivulet-gui"
    if [[ "$platform" == "windows" ]]; then
        gui="$release/rivulet-gui.exe"
    fi
    [[ -f "$gui" ]] || die "missing build output $gui (did the release build finish?)"

    mkdir -p "$staging"

    case "$platform" in
        linux | macos)
            copy "$release/rivulet-gui" "$staging/rivulet-gui"
            strip_binary "$staging/rivulet-gui"
            # Strip first, then copy: both names must be the same bytes so the
            # bare-binary asset and the packaging input cannot diverge.
            cp "$staging/rivulet-gui" "$staging/rivulet-$platform-$arch-gui"
            ;;
        windows)
            copy "$release/rivulet-gui.exe" "$staging/rivulet-gui.exe"
            copy "$release/rivulet-launcher.exe" "$staging/rivulet.exe"
            copy "$release/rivulet-updater.exe" "$staging/rivulet-updater.exe"
            ;;
    esac

    local name
    for name in "$staging"/*; do
        if [[ -f "$name" ]]; then
            printf 'staged %s (%s bytes)\n' "${name#"$staging"/}" "$(wc -c <"$name" | tr -d ' ')"
        fi
    done
    return 0
}

# â”€â”€ self-test â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

self_test() {
    set +e
    local failures=0
    local root
    root="$(mktemp -d)"
    local fake_strip="$root/strip"
    local strip_log="$root/strip.log"

    # A fake strip that leaves a marker behind: the test can then prove the
    # staged file was stripped, without depending on a real ELF/Mach-O binary.
    cat >"$fake_strip" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$1" >>"$(dirname "$0")/strip.log"
printf 'stripped' >>"$1"
EOF
    chmod +x "$fake_strip"

    # The fixtures are ASCII, so a shell-native comparison is exact and does not
    # depend on `cmp` being present (it is not part of every Git Bash install).
    same_fixture() { [[ "$(<"$1")" == "$(<"$2")" ]]; }

    check() {
        local condition="$1" message="$2"
        if [[ "$condition" != "0" ]]; then
            printf 'self-test FAILED: %s\n' "$message" >&2
            failures=$((failures + 1))
        fi
    }

    # `ok <message> <command...>` and `fails <message> <command...>`: every
    # check states what it asserts and against which status, instead of
    # asserting whichever command happened to run last (shellcheck
    # SC2319/SC2181).
    ok() {
        local message="$1"
        shift
        if "$@"; then
            return 0
        fi
        printf 'self-test FAILED: %s\n' "$message" >&2
        failures=$((failures + 1))
        return 1
    }
    fails() {
        local message="$1"
        shift
        local output status
        output="$("$@" 2>&1)"
        status=$?
        if [[ "$status" -eq 0 ]]; then
            printf 'self-test FAILED: %s\n' "$message" >&2
            # The invocation that should have been refused printed this.
            if [[ -n "$output" ]]; then
                printf '  command output: %s\n' "$output" >&2
            fi
            failures=$((failures + 1))
            return 1
        fi
        return 0
    }

    fixture() {
        local triple="$1"
        local dir="$root/target/$triple/release"
        mkdir -p "$dir"
        printf 'gui-bytes' >"$dir/rivulet-gui"
        printf 'launcher-bytes' >"$dir/rivulet-launcher.exe"
        printf 'gui-exe-bytes' >"$dir/rivulet-gui.exe"
        printf 'updater-bytes' >"$dir/rivulet-updater.exe"
    }

    local linux_triple="x86_64-unknown-linux-gnu"
    local macos_triple="aarch64-apple-darwin"
    local windows_triple="x86_64-pc-windows-msvc"
    fixture "$linux_triple"
    fixture "$macos_triple"
    fixture "$windows_triple"

    run() { bash "$0" "$@" >"$root/out.log" 2>&1; }

    # 1. Linux: the platform is derived from the triple, strip runs, and the
    #    qualified copy is a copy of the stripped file.
    ok "the Linux stage must succeed" run --target "$linux_triple" \
        --target-dir "$root/target" --staging "$root/linux" --strip "$fake_strip"
    ok "Linux must stage rivulet-gui" test -f "$root/linux/rivulet-gui"
    ok "Linux must stage the platform-qualified copy" test -f "$root/linux/rivulet-linux-x86_64-gui"
    ok "Linux must strip the staged binary" grep -q 'stripped' "$root/linux/rivulet-gui"
    ok "the qualified copy must be byte-identical to the stripped binary" same_fixture \
        "$root/linux/rivulet-gui" "$root/linux/rivulet-linux-x86_64-gui"
    ok "strip must run exactly once (the qualified copy must not be restripped)" test \
        "$(wc -l <"$strip_log" | tr -d ' ')" -eq 1
    ok "strip must run on the file that is shipped, not on the source" grep -q "linux/rivulet-gui" "$strip_log"

    # 2. macOS uses the same layout with the macOS qualifier.
    ok "the macOS stage must succeed" run --target "$macos_triple" \
        --target-dir "$root/target" --staging "$root/macos" --strip "$fake_strip"
    ok "macOS must use macos-aarch64 in the qualified name" test -f \
        "$root/macos/rivulet-macos-aarch64-gui"
    ok "macOS must strip its own payload exactly once" test \
        "$(wc -l <"$strip_log" | tr -d ' ')" -eq 2

    # 3. Windows stages three binaries and never strips.
    ok "the Windows stage must succeed" run --target "$windows_triple" \
        --target-dir "$root/target" --staging "$root/windows"
    for name in rivulet-gui.exe rivulet.exe rivulet-updater.exe; do
        ok "Windows must stage $name" test -f "$root/windows/$name"
    done
    ok "Windows must stage the launcher as rivulet.exe" test \
        "$(<"$root/windows/rivulet.exe")" = "launcher-bytes"
    ok "Windows must not strip (no strip call)" test \
        "$(wc -l <"$strip_log" | tr -d ' ')" -eq 2

    # 4. Staging twice from the same tree produces the same bytes: the step
    #    itself must not be a source of run-to-run drift.
    ok "staging twice must succeed" run --target "$linux_triple" \
        --target-dir "$root/target" --staging "$root/linux-again" --strip "$fake_strip"
    ok "re-staging the same input must be byte-identical" same_fixture \
        "$root/linux/rivulet-gui" "$root/linux-again/rivulet-gui"

    # 5. A missing build output is a failure that names the file, not an empty
    #    staging directory that fingerprints as "nothing to compare".
    fails "a missing build output must fail" run --target "$linux_triple" \
        --target-dir "$root/empty-target" --staging "$root/missing" --strip none
    ok "a failed stage must not leave a partial payload behind" test ! -e \
        "$root/missing/rivulet-gui"
    ok "the failure must say the build output is missing" grep -q 'missing build output' \
        "$root/out.log"

    # 6. Unusable invocations are refused rather than half-staged.
    fails "a platform that contradicts the triple must be refused" run --platform solaris \
        --target "$linux_triple" --target-dir "$root/target" --staging "$root/solaris" --strip none
    fails "a missing target must be refused" run --target "" \
        --target-dir "$root/target" --staging "$root/notarget"
    fails "a target with an unknown architecture must be refused" run \
        --target riscv64gc-unknown-linux-gnu --target-dir "$root/target" --staging "$root/riscv"
    fails "a target that maps to no release platform must be refused" run \
        --target x86_64-unknown-freebsd --target-dir "$root/target" --staging "$root/freebsd" --strip none

    # 7. `--strip auto` with no strip on PATH must fail, not silently ship an
    #    unstripped binary that differs from every previous release.
    local bash_path
    bash_path="$(command -v bash)"
    PATH="$root/empty-bin" "$bash_path" "$0" \
        --target "$linux_triple" --target-dir "$root/target" \
        --staging "$root/nostrip" >"$root/nostrip.log" 2>&1
    status=$?
    ok "auto strip without strip in PATH must fail loudly" test "$status" -ne 0
    ok "the failure must explain the missing strip" grep -qi 'no strip in PATH' "$root/nostrip.log"
    ok "the strip check must run before anything is staged" test ! -f "$root/nostrip/rivulet-gui"

    rm -rf "$root"
    if [[ "$failures" -ne 0 ]]; then
        printf '%d self-test check(s) failed\n' "$failures" >&2
        return 1
    fi
    printf 'stage-release-payload self-test: OK\n'
    return 0
}

main() {
    local platform="" target="${TARGET:-}" target_dir="${CARGO_TARGET_DIR:-target}"
    local staging="staging"
    strip_mode="${RIVULET_STRIP:-auto}"

    while [[ $# -gt 0 ]]; do
        case "$1" in
            --platform) platform="$2"; shift 2 ;;
            --target) target="$2"; shift 2 ;;
            --target-dir) target_dir="$2"; shift 2 ;;
            --staging) staging="$2"; shift 2 ;;
            --strip) strip_mode="$2"; shift 2 ;;
            --self-test) self_test; return $? ;;
            -h | --help) sed -n '2,45p' "$0" | sed 's/^# \{0,1\}//'; return 0 ;;
            *) die "unknown argument $1" ;;
        esac
    done

    stage
}

main "$@"
