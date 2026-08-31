# Recording formats & crash-safe remux (M4)

This documents the M4 container-format and remux work: additional recording
containers beyond MP4 and lossless post-stop remux to MP4 (the OBS workflow).

## Why crash-safe containers matter

MP4 stores its index (the `moov` box) **at the end of the file**. If the app
crashes mid-write, the file is unreadable and the whole recording is lost.

MKV, MOV, and MPEG-TS stream their index (or tolerate truncation), so an
interrupted write stays playable up to the last complete packet. OBS therefore
records to MKV by default and *remuxes* to MP4 after stopping.

## Containers

`RecordingContainer` (`rivulet-core`, `container.rs`) models the four formats:

| Container | GStreamer muxer | Extension | Crash-safe |
|-----------|-----------------|-----------|------------|
| MP4       | `mp4mux`        | `mp4`     | No         |
| MKV       | `matroskamux`   | `mkv`     | Yes        |
| MOV       | `qtmux`         | `mov`     | Yes        |
| TS        | `mpegtsmux`     | `ts`      | Yes        |

The MP4 default preserves the codec-native muxer — H.264/H.265 keep `mp4mux`
and VP9 keeps `webmmux` — so existing behavior is unchanged unless a user
explicitly chooses a crash-safe intermediate. The GUI exposes a container
picker next to the video-codec picker.

## Remux (issue #71)

`RemuxPlan` validates that a source container is a crash-safe intermediate and
builds the lossless remux pipeline fragment (demux -> identity copy -> mux,
**never re-encodes**):

```
filesrc location=in.mkv ! matroskademux name=demux demux. ! queue ! mp4mux name=mux mux. ! filesink location=out.mp4
```

`RemuxSettings` carries `auto_remux_after_stop` (default on, mirroring OBS) and
the target container (MP4). The source extension is swapped for the output.

## Status & follow-up

- [x] Container model + validation + GUI picker
- [x] Remux plan/settings + pipeline fragment
- [ ] GStreamer remux *execution* (calling the fragment after stop)

The GStreamer-side remux execution and recording file management (split
by time/size) are separate integration follow-ups.