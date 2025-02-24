#![allow(unused_imports)]
#![allow(dead_code)]
#![allow(unused_variables)]
use nix::fcntl::OFlag;
use nix::{sys::stat::Mode, *};
use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::{env, error::Error, result};

type MyResult<T> = result::Result<T, Box<dyn Error>>;

fn scan_directory(dir: &Path) -> HashMap<PathBuf, String> {
    //Returns Full path, i.e.. PathBuf and a string saying that its a file or directory
    let open_dir = nix::dir::Dir::open(dir, OFlag::O_RDONLY, Mode::empty()).unwrap();
    let mut output = HashMap::new();
    for res in open_dir {
        match res {
            Ok(entry) => {
                let full_path = dir.join(entry.file_name().to_string_lossy().as_ref());
                let file_type = match entry.file_type() {
                    Some(nix::dir::Type::Directory) => "dir".to_string(),
                    Some(nix::dir::Type::File) => "file".to_string(),
                    _ => "unknown".to_string(),
                };
                output.insert(full_path, file_type);
            }
            Err(e) => eprintln!("Error {}", e),
        }
    }
    output
}

fn get_file_size_in_bytes(file_path: &Path) -> i64 {
    //Returns file size in bytes
    if let Ok(res) = nix::sys::stat::stat(file_path) {
        res.st_size
    } else {
        0
    }
}

fn get_file_size(file_path: &Path) -> String {
    //Returns file size in human readable format
    if let Ok(res) = nix::sys::stat::stat(file_path) {
        let bytes = res.st_blocks * 512;
        if bytes < 1024 {
            format!("{}", bytes)
        } else if bytes >= 1024 && bytes < 1048576 {
            format!("{:.1}K", bytes as f64 / 1024.0)
        } else if bytes >= 1048576 && bytes < 1073741824 {
            format!("{:.1}M", bytes as f64 / 1048576.0)
        } else {
            format!("{:.1}G", bytes as f64 / 1073741824.0)
        }
    } else {
        0.to_string()
    }
}
fn format_file_size(file_path: &Path, arg: String) -> MyResult<String> {
    //Returns file size based on argument given
    if let Ok(res) = nix::sys::stat::stat(file_path) {
        let bytes = res.st_blocks * 512;
        let output = match arg.as_str() {
            "BK" => format!("{}K", (bytes as f64 / 1024.0).ceil()),
            "BM" => format!("{}M", (bytes as f64 / 1048576.0).ceil()),
            "BG" => format!("{}G", (bytes as f64 / 1073741824.0).ceil()),
            _ => return Err("-B Requires an Argument".into()),
        };
        return Ok(output);
    }
    Err("Failed to get file size".into())
}

fn count_files(dir: &Path) -> u64 {
    todo!()
}

fn calculate_total_dir_size(dir: &Path) -> u64 {
    todo!()
}

fn main() {
    let current_dir = env::current_dir().unwrap();
    //let output = nix::dir::Dir::open(&current_dir, OFlag::O_RDONLY, Mode::empty()).unwrap();
    //for res in output {
    //    match res {
    //        Ok(e) => {
    //            println!("{}, inode: {}", e.file_name().to_string_lossy(), e.ino());
    //            match e.file_type() {
    //                Some(file) => {
    //                    println!("This is a: {:?}", file);
    //                }
    //                None => println!("wth is this!"),
    //            }
    //        }
    //        Err(e) => eprintln!("Error {}", e),
    //    }
    //}

    let output = scan_directory(&current_dir);
    for (path, file_type) in output {
        if file_type == "file" {
            let file_size = get_file_size_in_bytes(&path);
            println!(
                "filename: {:?} filesize: {file_size} bytes",
                path.file_name()
            );
            let human_readable_size = get_file_size(&path);
            println!("{human_readable_size}");
            let output = format_file_size(&path, "".to_string());
            println!("{:?}", output);
        }
    }
}
