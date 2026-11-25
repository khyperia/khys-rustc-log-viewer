use std::{net::ToSocketAddrs as _, path::PathBuf};

use crate::parser::{DataSource, SocketDataSource};

pub fn parse_cmdline() -> Option<DataSource> {
    let mut args = std::env::args().skip(1);
    if let Some(first) = args.next() {
        if let Some(second) = args.next() {
            if first == "--tcp" {
                match second.to_socket_addrs() {
                    Ok(socket_addr) => {
                        let fork_after_listen = args.collect();
                        let socket = SocketDataSource {
                            socket_addr,
                            fork_after_listen,
                        };
                        Some(DataSource::Socket(socket))
                    }
                    Err(e) => {
                        println!("couldn't parse tcp address: {e}");
                        None
                    }
                }
            } else {
                println!(
                    "Only one argument is allowed (the path that was given to RUSTC_LOG_OUTPUT_TARGET)"
                );
                None
            }
        } else {
            Some(DataSource::File(PathBuf::from(first)))
        }
    } else {
        if let Ok(targ) = std::env::var("RUSTC_LOG_OUTPUT_TARGET") {
            Some(DataSource::File(PathBuf::from(targ)))
        } else {
            println!("No file provided. Pass the file that was given to RUSTC_LOG_OUTPUT_TARGET");
            None
        }
    }
}
