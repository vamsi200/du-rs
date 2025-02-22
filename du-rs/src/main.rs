#![allow(unused_imports)]
#![allow(dead_code)]
#![allow(unused_variables)]
use nix::fcntl::OFlag;
use nix::{sys::stat::Mode, *};
use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::{env, error::Error};

fn scan_directory(dir: &Path) -> HashMap<PathBuf, String> {
    todo!()
}

fn get_file_size_in_bytes(file_path: &Path) -> u64 {
    todo!()
}

fn format_file_size(bytes: u64) -> String {
    todo!()
}

fn count_files(dir: &Path) -> u64 {
    todo!()
}

fn calculate_total_dir_size(dir: &Path) -> u64 {
    todo!()
}

fn main() {
    let current_dir = env::current_dir().unwrap();
    let output = nix::dir::Dir::open(&current_dir, OFlag::O_RDONLY, Mode::empty()).unwrap();
    for res in output {
        match res {
            Ok(e) => {
                println!("{}, inode: {}", e.file_name().to_string_lossy(), e.ino());
                match e.file_type() {
                    Some(file) => {
                        println!("This is a: {:?}", file);
                    }
                    None => println!("wth is this!"),
                }
            }
            Err(e) => eprintln!("Error {}", e),
        }
    }
}
