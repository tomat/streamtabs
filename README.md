# streamtabs

`streamtabs` helps you watch live terminal output in filter tabs.

You pipe text in, pass filters as arguments, and get:

- tab `0`: `(all)` lines
- tabs `1..N`: one tab per filter

This makes it easy to follow noisy logs without rerunning commands.

## Quick Start

Build once:

```bash
cargo build --release
```

Run with at least one filter:

```bash
tail -f app.log | ./target/release/streamtabs error warn info
```

## More Examples

```bash
cat ./file.txt | ./target/release/streamtabs foo bar
```

```bash
log stream --style compact | ./target/release/streamtabs Error Fault WindowServer
```

## Controls

- `Tab`: next tab
- `0` to `9`: jump to tab number
- `Space`: pause/resume
- `q` or `Ctrl+C`: quit
- Mouse click tab: switch tabs
- Mouse click line: highlight that line across tabs
- `d`: cancel highlighted line

## Notes

- Run in a terminal (`stdout` must be a TTY).
- `streamtabs` requires at least one filter argument.
- Each tab stores up to `5000` lines.

## Screenshots

Live stream:

![Live stream screenshot](docs/screenshots/live.png)

Filtered tab selected (`3` = `INFO`):

![Filtered tab screenshot](docs/screenshots/filtered.png)

Selected line on `ERROR` tab:

![Selected screenshot](docs/screenshots/selected.png)

Selected `ERROR` line after switching to `DEBUG` tab, then pausing:

![Selected paused switched screenshot](docs/screenshots/selected-paused-switched.png)
