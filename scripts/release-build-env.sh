#!/usr/bin/env bash
# Deterministic build inputs for release artifacts (issue #288).
#
# Two independent builds of the same commit only produce the same bytes if the
# build inputs are the same. Three of them are not, unless they are normalized:
#
#   * the checkout path  -> embedded in debug info and panic locations
#   * the Cargo home     -> registry source paths of every dependency
#   * the link timestamp -> PE TimeDateStamp and the synthetic PDB signature
#
# This script is the single place that defines the normalized values. The
# release build (`build-package.yml`) and the rebuild-and-compare job
# (`reproducible-build.yml`) both consume it, so the comparison cannot silently
# measure a different recipe than the one that ships. A ci_pinning guard fails
# when either workflow stops calling it.
#
# `SOURCE_DATE_EPOCH` is the commit timestamp, not "now": it is the standard
# reproducible-build input, and a build script that embeds a date must get a
# value that is a property of the source, not of the runner's clock.
#
# Usage:
#   scripts/release-build-env.sh >> "$GITHUB_ENV"          # in a workflow
#   scripts/release-build-env.sh --print                    # inspect
#   scripts/release-build-env.sh --self-test
#
# Options exist for the self-test and for non-GitHub machines:
#   --runner-os Linux|macOS|Windows   default: $RUNNER_OS, else uname
#   --workspace PATH                  default: $GITHUB_WORKSPACE, else $PWD
#   --cargo-home PATH                 default: $CARGO_HOME, else ~/.cargo
#   --epoch SECONDS                   default: git log -1 --format=%ct
set -euo pipefail

# RUSTFLAGS is split on whitespace by Cargo, so a remap prefix containing a
# space cannot be expressed. Refusing is the honest failure: silently passing a
# truncated prefix produces a build that *looks* normalized and is not.
refuse_whitespace() {
    local label="$1" value="$2"
    case "$value" in
        *[[:space:]]*)
            printf 'release-build-env: %s %q contains whitespace; --remap-path-prefix cannot express it (RUSTFLAGS is whitespace-split). Move the checkout to a path without spaces.\n' \
                "$label" "$value" >&2
            return 1
            ;;
    esac
    return 0
}

strip_trailing_separators() {
    local value="$1"
    while [[ "$value" == */ || "$value" == *\\ ]]; do
        value="${value%?}"
    done
    printf '%s' "$value"
}

# Git Bash on a Windows runner reports POSIX paths (`/d/a/Rivulet/Rivulet`) while
# rustc embeds the native spelling (`D:\a\Rivulet\Rivulet`). Remapping the POSIX
# form silently matches nothing, so the build would look normalized and still
# differ. `cygpath` is the only reliable translation.
#
# The translation is gated on the *target* OS, not on the presence of `cygpath`:
# Git Bash ships `cygpath` on every Windows runner, so keying off the binary
# rewrote a Linux path (`/home/runner/...`) into `C:\Program Files\Git\home\...`
# -- with a space, which the whitespace check then refused. On Linux and macOS
# a POSIX path is already native and must be passed through untouched.
to_native() {
    local runner_os="$1" value="$2"
    if [[ "$runner_os" == "Windows" ]]; then
        case "$value" in
            /*)
                if command -v cygpath >/dev/null 2>&1; then
                    cygpath -w "$value"
                    return
                fi
                ;;
        esac
    fi
    printf '%s' "$value"
}

compute() {
    RUNNER_OS_VALUE="$runner_os"
    WORKSPACE_VALUE="$(to_native "$runner_os" "$(strip_trailing_separators "$workspace")")"
    CARGO_HOME_VALUE="$(to_native "$runner_os" "$(strip_trailing_separators "$cargo_home")")"

    if [[ -z "$WORKSPACE_VALUE" ]]; then
        printf 'release-build-env: no workspace path given (set --workspace or GITHUB_WORKSPACE)\n' >&2
        return 1
    fi
    if [[ -z "$CARGO_HOME_VALUE" ]]; then
        printf 'release-build-env: no Cargo home given (set --cargo-home or CARGO_HOME)\n' >&2
        return 1
    fi
    refuse_whitespace "workspace" "$WORKSPACE_VALUE"
    refuse_whitespace "Cargo home" "$CARGO_HOME_VALUE"

    if [[ -z "$epoch" ]]; then
        epoch="$(git log -1 --format=%ct 2>/dev/null || true)"
    fi
    if [[ ! "$epoch" =~ ^[0-9]+$ ]]; then
        printf 'release-build-env: no commit timestamp (pass --epoch or run inside the repository)\n' >&2
        return 1
    fi

    remap="$WORKSPACE_VALUE=/rivulet $CARGO_HOME_VALUE=/cargo"
    link_args=""
    # MSVC's link.exe defaults to a wall-clock TimeDateStamp and a PDB
    # signature that changes on every link; `/Brepro` derives the timestamp
    # from the inputs and `/DEBUG:NONE` drops the synthetic PDB reference
    # (the release never publishes a PDB, so the reference pointed at a file
    # nobody has). Measured: without both flags two builds 3 s apart differ.
    if [[ "$RUNNER_OS_VALUE" == "Windows" ]]; then
        link_args="/Brepro /DEBUG:NONE"
    fi

    rustflags=""
    for pair in $remap; do
        rustflags="$rustflags --remap-path-prefix=$pair"
    done
    for arg in $link_args; do
        rustflags="$rustflags -C link-arg=$arg"
    done
    rustflags="${rustflags# }"

    printf 'SOURCE_DATE_EPOCH=%s\n' "$epoch"
    printf 'REPRODUCIBLE_REMAP=%s\n' "$remap"
    printf 'REPRODUCIBLE_LINK_ARGS=%s\n' "$link_args"
    printf 'RUSTFLAGS=%s\n' "$rustflags"
}

detect_runner_os() {
    if [[ -n "${RUNNER_OS:-}" ]]; then
        printf '%s' "$RUNNER_OS"
        return
    fi
    case "$(uname -s 2>/dev/null || echo unknown)" in
        Darwin) printf 'macOS' ;;
        MINGW*|MSYS*|CYGWIN*) printf 'Windows' ;;
        *) printf 'Linux' ;;
    esac
}

self_test() {
    set +e
    local failures=0
    local tmp
    tmp="$(mktemp -d)"

    # `ok <message> <command...>` and `fails <message> <command...>`: every
    # check states what it asserts and against which status. The previous
    # `check "$?"` form asserted whichever command happened to run last, which
    # is how a self-test starts passing for the wrong reason (shellcheck calls
    # that SC2319/SC2181).
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
    # A herestring is shell syntax, not an argument, so a check that greps the
    # captured output needs a wrapper. `contains` is a regex, `contains_fixed`
    # a literal.
    contains() {
        grep -Eq -- "$2" <<<"$1"
    }
    contains_fixed() {
        grep -Fq -- "$2" <<<"$1"
    }

    local out status

    # Linux: two remaps, no linker flags.
    out="$(bash "$0" --runner-os Linux --workspace /home/runner/work/Rivulet/Rivulet \
        --cargo-home /home/runner/.cargo --epoch 1762000000)"
    ok "the env must be exactly four KEY=VALUE lines" test "$(grep -c . <<<"$out")" -eq 4
    ok "the commit timestamp must be exported verbatim" contains "$out" '^SOURCE_DATE_EPOCH=1762000000$'
    ok "both the workspace and the Cargo home must be remapped" contains_fixed "$out" \
        'REPRODUCIBLE_REMAP=/home/runner/work/Rivulet/Rivulet=/rivulet /home/runner/.cargo=/cargo'
    ok "RUSTFLAGS must carry one --remap-path-prefix per input" contains "$out" \
        '^RUSTFLAGS=--remap-path-prefix=/home/runner/work/Rivulet/Rivulet=/rivulet --remap-path-prefix=/home/runner/.cargo=/cargo$'
    fails "Linux must not pass MSVC linker flags" contains_fixed "$out" 'Brepro'
    # No quoting: $GITHUB_ENV stores the literal value, so quotes would become
    # part of RUSTFLAGS and every rustc invocation would fail.
    fails "the output must not be shell-quoted" contains "$out" "['\"]"

    # Windows: adds the two MSVC linker flags that make PE output reproducible.
    out="$(bash "$0" --runner-os Windows --workspace 'D:\a\Rivulet\Rivulet' \
        --cargo-home 'C:\Users\runneradmin\.cargo' --epoch 1762000000)"
    ok "Windows must pass /Brepro (deterministic PE timestamp)" contains_fixed "$out" \
        '-C link-arg=/Brepro'
    ok "Windows must drop the nondeterministic PDB reference" contains_fixed "$out" \
        '-C link-arg=/DEBUG:NONE'

    # A prefix with a space cannot be expressed in RUSTFLAGS: refuse instead of
    # silently emitting a truncated prefix.
    bash "$0" --runner-os Linux --workspace '/home/runner/my repo' \
        --cargo-home /root/.cargo --epoch 1 >"$tmp/space.out" 2>"$tmp/space.err"
    status=$?
    ok "a workspace path with a space must be refused" test "$status" -ne 0
    ok "the refusal must name the whitespace problem" grep -Fiq 'whitespace' "$tmp/space.err"

    # Missing inputs must fail, not produce a half-normalized build.
    fails "an empty workspace must be refused" bash "$0" --runner-os Linux \
        --workspace '' --cargo-home /root/.cargo --epoch 1
    fails "an empty Cargo home must be refused" bash "$0" --runner-os Linux \
        --workspace /work --cargo-home '' --epoch 1
    fails "a non-numeric timestamp must be refused" bash "$0" --runner-os Linux \
        --workspace /work --cargo-home /root/.cargo --epoch 'not-a-number'

    # Trailing separators are a property of the caller, not of the build.
    out="$(bash "$0" --runner-os Linux --workspace /work/repo/ --cargo-home /root/.cargo/ --epoch 5)"
    ok "trailing separators must not leak into the remap prefixes" contains_fixed "$out" \
        'REPRODUCIBLE_REMAP=/work/repo=/rivulet /root/.cargo=/cargo'

    # On a Windows runner Git Bash hands out POSIX paths while rustc embeds the
    # native spelling. A remap against the POSIX form matches nothing and the
    # build silently stays non-reproducible, so the translation is mandatory
    # wherever cygpath exists. A fake cygpath proves the branch without needing
    # a Windows host, and a native path must not be passed through it.
    local fake_bin="$tmp/fake-bin"
    mkdir -p "$fake_bin"
    cat >"$fake_bin/cygpath" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$2" >>"$(dirname "$0")/calls.log"
printf 'WIN(%s)' "${2#/}"
EOF
    chmod +x "$fake_bin/cygpath"

    # The translation is Windows-only. Keying it off `cygpath` alone rewrote the
    # Linux case above on the Windows runner (`/home/runner/...` became
    # `C:\Program Files\Git\home\...`, whose space the whitespace check then
    # refused), so a fake cygpath on PATH must leave a non-Windows runner alone.
    out="$(PATH="$fake_bin:$PATH" bash "$0" --runner-os Linux --workspace /home/runner/work/Rivulet/Rivulet \
        --cargo-home /home/runner/.cargo --epoch 9)"
    ok "a non-Windows runner must leave POSIX paths untouched" contains_fixed "$out" \
        'REPRODUCIBLE_REMAP=/home/runner/work/Rivulet/Rivulet=/rivulet /home/runner/.cargo=/cargo'
    ok "a non-Windows runner must not call cygpath" test ! -e "$fake_bin/calls.log"

    out="$(PATH="$fake_bin:$PATH" bash "$0" --runner-os Windows --workspace /d/a/Rivulet/Rivulet \
        --cargo-home /c/Users/runneradmin/.cargo --epoch 9)"
    ok "POSIX paths on a Windows runner must be translated to native ones" contains_fixed "$out" \
        'REPRODUCIBLE_REMAP=WIN(d/a/Rivulet/Rivulet)=/rivulet WIN(c/Users/runneradmin/.cargo)=/cargo'
    ok "both the workspace and the Cargo home must be translated" test \
        "$(wc -l <"$fake_bin/calls.log" | tr -d ' ')" -eq 2

    out="$(PATH="$fake_bin:$PATH" bash "$0" --runner-os Windows --workspace 'D:\a\Rivulet\Rivulet' \
        --cargo-home 'C:\Users\runneradmin\.cargo' --epoch 9)"
    ok "native Windows paths must be used verbatim" contains_fixed "$out" \
        'REPRODUCIBLE_REMAP=D:\a\Rivulet\Rivulet=/rivulet C:\Users\runneradmin\.cargo=/cargo'
    ok "native Windows paths must not be passed through cygpath" test \
        "$(wc -l <"$fake_bin/calls.log" | tr -d ' ')" -eq 2

    if [[ "$failures" -ne 0 ]]; then
        printf '%d self-test check(s) failed\n' "$failures" >&2
        rm -rf "$tmp"
        return 1
    fi
    rm -rf "$tmp"
    printf 'release-build-env self-test: OK\n'
}

main() {
    local runner_os="" workspace="" cargo_home="" epoch=""
    # An explicitly empty option must reach compute() and be refused there; a
    # `:-` default would silently substitute the environment and produce a
    # build that looks normalized against the wrong path.
    local workspace_given=0 cargo_home_given=0
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --runner-os) runner_os="$2"; shift 2 ;;
            --workspace) workspace="$2"; workspace_given=1; shift 2 ;;
            --cargo-home) cargo_home="$2"; cargo_home_given=1; shift 2 ;;
            --epoch) epoch="$2"; shift 2 ;;
            --print) shift ;;
            --self-test) self_test; return $? ;;
            -h|--help) sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//'; return 0 ;;
            *) printf 'release-build-env: unknown argument %s\n' "$1" >&2; return 2 ;;
        esac
    done

    runner_os="${runner_os:-$(detect_runner_os)}"
    if [[ "$workspace_given" -eq 0 ]]; then
        workspace="${GITHUB_WORKSPACE:-$PWD}"
    fi
    if [[ "$cargo_home_given" -eq 0 ]]; then
        cargo_home="${CARGO_HOME:-$HOME/.cargo}"
    fi
    compute
}

main "$@"
