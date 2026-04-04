Khy's rustc log viewer
===

Under heavy development and very much a work in progress (unless I stop working on it and forget to update this
readme...)

This is a viewer app for [`RUSTC_LOG`](https://rustc-dev-guide.rust-lang.org/tracing.html), intended for development on
rustc. The default format omits several extremely useful bits of information that you typically don't want to see, but
sometimes you do, preferably without running rustc with different log flags. So, this is a GUI app that lets you
dynamically view/hide/filter `RUSTC_LOG` information.

Developed as a personal project, not currently intended to be used by others, but published in case someone wants to
either use it or take inspiration from it (MIT licensed). Please send me a message if you find this useful, I'd love to
hear from you!

# Usage

Export `RUSTC_LOG_FORMAT_JSON=1` and set `RUSTC_LOG_OUTPUT_TARGET` to a file path. Pass that file path into this
program, or leave `RUSTC_LOG_OUTPUT_TARGET` exported and run this program without args (it reads that var if there are
no arguments).

Personally, I use this [fish](https://fishshell.com/) function, shoved into `~/.config/fish/functions/viewlog.fish`:

```fish
function viewlog --wraps='rustc +stage1'
    if test -d $PWD/target/viewlog-output
        rm -r $PWD/target/viewlog-output
    end
    if test ! -p /tmp/viewlog
        mkfifo /tmp/viewlog
    end
    cargo run --release --manifest-path ~/me/khys-rustc-log-viewer/Cargo.toml -- /tmp/viewlog &
    set -fx RUSTC_LOG_FORMAT_JSON 1
    set -fx RUSTC_LOG_OUTPUT_TARGET /tmp/viewlog
    if ! set -q RUSTC_LOG
        set -fx RUSTC_LOG trace
    end
    rustc +stage1 --out-dir $PWD/target/viewlog-output $argv
end
```

- creates a named pipe, `/tmp/viewlog`, to not actually need to store the json on disk (it can be several gigabytes)
    - idk if a similar concept exists on windows - if you're using a regular file though (totally fine to do), you'll
      need to wait for rustc to finish running before opening this log viewer
- sets up the `RUSTC_LOG_FORMAT_JSON` and `RUSTC_LOG_OUTPUT_TARGET` vars
- allows you to specify a filter for `RUSTC_LOG`, defaulting to trace if not set (note that if the log viewer hasn't
  compiled yet, `RUSTC_LOG` will apply to the viewer...)
- presumes you've [set up `+stage1`](https://rustc-dev-guide.rust-lang.org/building/how-to-build-and-run.html#creating-a-rustup-toolchain)
- passes through $argv to rustc, presumably a path to a UI test

Once the app is running, uuuh, it's a bit of a WIP mess right now, there's vaguely less/vim-like keybindings I guess

# I need to apologize

I handrolled my own json parser for this (sorry...). That means that if you try to use this, it might be kinda brittle.
The reason I did so is because the handrolled version is ~9x faster than facet\_json, which is the difference between
waiting 14 seconds and waiting 1.5 seconds to parse a 1.5GiB file (typical for RUSTC\_LOG=trace on a basic ui test). The
custom parser mostly assumes the input is well-formed, it doesn't do sanity checks like duplicate field detection.
Hopefully, if the format changes, it's relatively easy to patch up the parser!
