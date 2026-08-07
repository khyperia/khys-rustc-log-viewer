Khy's rustc log viewer
===

This is a viewer app for [`RUSTC_LOG`](https://rustc-dev-guide.rust-lang.org/tracing.html), intended for development on
rustc. The default format omits several extremely useful bits of information that you typically don't want to see, but
sometimes you do, preferably without running rustc with different log flags. So, this is a GUI app that lets you
dynamically view/hide/filter `RUSTC_LOG` information.

Developed as a personal project mostly for my own use, but published in case someone wants to either use it or take
inspiration from it (MIT licensed). It's fairly stable at this point and I'm using it pretty regularly for my work.
Please send me a message if you find this useful, I'd love to hear from you!

# Usage

Export `RUSTC_LOG_FORMAT_JSON=1` and set `RUSTC_LOG_OUTPUT_TARGET` to a file path. Pass that file path into this
program, or leave `RUSTC_LOG_OUTPUT_TARGET` exported and run this program without args (it reads that var if there are
no arguments).

Personally, I use this [fish](https://fishshell.com/) function, shoved into `~/.config/fish/functions/viewlog.fish`:

```fish
function viewlog --wraps='rustc +stage1'
    if test -d $PWD/build/viewlog-output
        rm -r $PWD/build/viewlog-output
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
    rustc +stage1 --out-dir $PWD/build/viewlog-output $argv
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

Once the app is running, uuuh, it's a bit of a WIP mess, there's vaguely less/vim-like keybindings I guess

# Remote usage

This is a bit advanced, sorry. There's another format the log viewer can run in, with the `--tcp` argument. This hosts a
tcp listener, and uses the data that comes over it as the data stream. After the `--tcp` argument is an optional command
to fork after the tcp listener has successfully set up. I use it like this (replace `$argv` with the ssh host you want
to connect to):

```fish
cargo run --release --manifest-path ~/me/khys-rustc-log-viewer/Cargo.toml -- --tcp localhost:12543 ssh -T -C -R 12543:localhost:12543 $argv 'socat -u PIPE:/tmp/viewlog TCP:localhost:12543'
```

This command does the following:

- Runs the log viewer, hosting a tcp listener on `localhost:12543`
- Then, connects to a remote host via ssh
  - the -T argument disables tty allocation (idk if it's needed, shrug)
  - the -C argument enables compression (extremely important, this reduces the amount of data sent by over 95%)
  - the `-R 12543:localhost:12543` argument forwards any connection to `localhost:12543` on the remote host to `localhost:12543` on the local host
- Then, on the remote host, socat is used to translate from a named pipe to a TCP connection (socat must be installed on the remote host)
  - the -u argument is for "unidirectional" (only go from the pipe to the tcp socket, not the other way around)
    - this is required to make closing the tcp socket work properly when the named pipe has an EOF
  - fyi: socat automatically creates `/tmp/viewlog` as a named pipe if it doesn't already exist :3

Once this command is running, you may run rustc on the host with `RUSTC_LOG_OUTPUT_TARGET=/tmp/viewlog` and the log will
show up in your local log viewer GUI. I personally use a variant of my `viewlog` fish alias, but with the `cargo run
[...]` line removed, as well as replacing the `mkfifo` with a check if it exsts, printing an error and returning if not.

... I'd like the app to not be a oneshot, but rather be able to clear and reload data from running again... but bump
allocator lifetimes in a GUI app are hard.

# I need to apologize

I handrolled my own json parser for this (sorry...). That means that if you try to use this, it might be kinda brittle.
The reason I did so is because the handrolled version is ~9x faster than facet\_json, which is the difference between
waiting 14 seconds and waiting 1.5 seconds to parse a 1.5GiB file (typical for RUSTC\_LOG=trace on a basic ui test). The
custom parser mostly assumes the input is well-formed, it doesn't do sanity checks like duplicate field detection.
Hopefully, if the format changes, it's relatively easy to patch up the parser!
