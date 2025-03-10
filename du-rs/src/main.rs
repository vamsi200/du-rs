#![allow(unused_imports)]
#![allow(dead_code)]
#![allow(unused_variables)]
use nix::fcntl::OFlag;
use nix::{sys::stat::Mode, *};
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::exit;
use std::{env, error::Error, result};

type Result<T> = result::Result<T, Box<dyn Error>>;

struct FileSize {
    bytes: i64,
    formatted: String,
}

fn get_file_size_in_bytes(file_path: &PathBuf) -> i64 {
    nix::sys::stat::stat(file_path)
        .map(|res| res.st_size)
        .unwrap_or(0)
}
fn get_disk_usage_blocks(path: &PathBuf) -> i64 {
    nix::sys::stat::stat(path)
        .map(|res| (res.st_blocks * 512) / 1024)
        .unwrap_or(0)
}
fn get_disk_usage_bytes(path: &PathBuf) -> i64 {
    nix::sys::stat::stat(path)
        .map(|res| res.st_blocks * 512)
        .unwrap_or(0)
}

fn get_file_size(file_path: Option<&Path>, bytes: Option<i64>) -> FileSize {
    static UNITS: [&str; 3] = ["K", "M", "G"];
    static DIVISORS: [i64; 3] = [1024, 1048576, 1073741824];

    let bytes = if let Some(f) = file_path {
        nix::sys::stat::stat(f)
            .map(|res| res.st_blocks * 512)
            .unwrap_or(0)
    } else {
        bytes.unwrap_or(0)
    };
    let mut formatted = String::with_capacity(16);
    if bytes < 1024 {
        formatted.push_str(&format!("{}B", bytes));
    } else {
        let (value, unit) = UNITS
            .iter()
            .zip(DIVISORS.iter())
            .find(|&(_, &div)| bytes < div * 1024)
            .map(|(unit, &div)| ((bytes as f64) / (div as f64), unit))
            .unwrap_or(((bytes as f64) / 1073741824.0, &"G"));

        use std::fmt::Write;
        let _ = write!(formatted, "{:.1}{}", value, unit);
    }

    FileSize { bytes, formatted }
}
fn format_file_size(file_map: &BTreeMap<PathBuf, Vec<PathBuf>>, arg: &str) -> Result<String> {
    const UNITS: [(&str, f64); 7] = [
        ("K", 1024.0),
        ("M", 1_048_576.0),
        ("G", 1_073_741_824.0),
        ("T", 1_099_511_627_776.0),
        ("P", 1_125_899_906_842_624.0),
        ("E", 1_152_921_504_606_846_976.0),
        ("Z", 1_180_591_620_717_411_303_424.0),
    ];

    let mut output = String::with_capacity(16);

    for (dir_path, _) in file_map {
        let res = nix::sys::stat::stat(dir_path).map_err(|_| "Failed to get file size")?;
        let bytes = res.st_blocks * 512;

        if let Some((unit, divisor)) = UNITS.iter().find(|&&(u, _)| arg == format!("-B{}", u)) {
            let _ = write!(output, "{}{}", (bytes as f64 / divisor).ceil(), unit);
            return Ok(output);
        } else if let Ok(block_size) = arg[2..].parse::<i64>() {
            let adjusted_size = (bytes as f64 / block_size as f64).ceil() as i64;
            let _ = write!(output, "{}B", adjusted_size * block_size);
            return Ok(output);
        }
    }

    Err("-B Requires a valid argument".into())
}

fn calculate_total_dir_size<F>(
    dir: &BTreeMap<PathBuf, Vec<PathBuf>>,
    format: bool,
    is_bytes: bool,
    r_files: bool,
    mut output_fn: F,
) -> i64
where
    F: FnMut(&str),
{
    let c_dir = env::current_dir().expect("Failed to get current directory");
    let mut dir_sizes = BTreeMap::new();
    let mut output_buffer = String::with_capacity(256);
    let mut empty_file = String::new();
    let mut dir_size;
    let mut file_size;
    for (dir_path, file_names) in dir.iter() {
        if is_bytes {
            dir_size = 0;
        } else if format {
            dir_size = get_disk_usage_bytes(dir_path);
        } else {
            dir_size = get_disk_usage_blocks(dir_path);
        }
        for file in file_names {
            if !is_bytes && !format {
                file_size = get_disk_usage_blocks(file);
            } else if is_bytes {
                file_size = get_file_size_in_bytes(file);
            } else {
                file_size = get_disk_usage_bytes(file);
            }
            if file_size == 0 {
                if let Ok(rel_path) = file.strip_prefix(&c_dir) {
                    if let Some(first_part) = rel_path.to_str().unwrap().split('/').next() {
                        empty_file = first_part.to_string();
                    }
                }
            }
            dir_size += file_size;
            if r_files {
                output_buffer.clear();
                let relative_path = file.strip_prefix(&c_dir).unwrap_or(file);
                if format {
                    let formatted_size = get_file_size(None, Some(file_size));
                    if file == &c_dir {
                        write!(
                            output_buffer,
                            "{:<10} ./{}",
                            formatted_size.formatted,
                            relative_path.display()
                        )
                        .unwrap();
                    } else {
                        write!(
                            output_buffer,
                            "{:<10} {}",
                            formatted_size.formatted,
                            relative_path.display()
                        )
                        .unwrap();
                    }
                } else {
                    if file == &c_dir {
                        write!(
                            output_buffer,
                            "{:<10} ./{}",
                            file_size,
                            relative_path.display()
                        )
                        .unwrap();
                    } else {
                        write!(
                            output_buffer,
                            "{:<10} {}",
                            file_size,
                            relative_path.display()
                        )
                        .unwrap();
                    }
                }
                output_fn(&output_buffer);
            }
        }

        dir_sizes.insert(dir_path, dir_size);
    }

    let mut total_size = 0;
    let mut counted_files = HashSet::new();

    for (dir_path, file_names) in dir.iter() {
        if !is_bytes {
            if !format {
                total_size += get_disk_usage_blocks(dir_path);
            } else {
                total_size += get_disk_usage_bytes(dir_path);
            }
        }
        for file in file_names {
            if !counted_files.contains(file) {
                if is_bytes {
                    total_size += get_file_size_in_bytes(file);
                } else if format {
                    total_size += get_disk_usage_bytes(file);
                } else {
                    total_size += get_disk_usage_blocks(file);
                }
                counted_files.insert(file);
            }
        }
    }

    for (i, (&dir_path, &dir_size)) in dir_sizes.iter().rev().enumerate() {
        output_buffer.clear();
        let relative_path = dir_path.to_str().unwrap().trim_end_matches("/");
        let root_dir = relative_path.trim_start_matches("./");

        let display_size = if root_dir == empty_file {
            if format && r_files || format {
                dir_size + 4096
            } else {
                dir_size + (8 * 512) / 1024
            }
        } else {
            dir_size
        };
        if i == dir_sizes.len() - 1 {
            if format {
                let formatted_size = get_file_size(None, Some(total_size));
                write!(
                    output_buffer,
                    "{:<10} {}",
                    formatted_size.formatted, relative_path
                )
                .unwrap();
            } else {
                write!(output_buffer, "{:<10} {}", total_size, relative_path).unwrap();
            }
        } else {
            if format {
                let formatted_size = get_file_size(None, Some(display_size));
                write!(
                    output_buffer,
                    "{:<10} {}",
                    formatted_size.formatted, relative_path
                )
                .unwrap();
            } else {
                write!(output_buffer, "{:<10} {}", display_size, relative_path).unwrap();
            }
        }
        output_fn(&output_buffer);
    }
    total_size
}
fn print_help() {
    println!(
        "Usage: du-rs [OPTIONS] [PATH]
Options:
  -h, --help              Show this help message and exit
  -a, --all               Include hidden files
  -ah                     Include hidden files and use human-readable sizes
  -b                      Display sizes in bytes
  -s, --summarize         Summarize directory sizes
  -c, --total             Show total size
  -d, --max-depth DEPTH   Set maximum depth for directory traversal
  -B<size>                Set block size
  -t, --threshold VALUE   Set size threshold
  -x, --one-file-system PATH  Limit scanning to one file system
  -X, --exclude-from PATH    Exclude paths from a file"
    );
    exit(0);
}

#[derive(Debug)]
struct Args {
    path: PathBuf,
    human_readable: bool,
    depth: Option<i32>,
    summarize: bool,
    bytes: bool,
    total: bool,
    block_size: Option<String>,
    threshold: Option<u64>,
    x: Option<PathBuf>,
    xclude: Option<PathBuf>,
    a: bool,
}

fn handle_args() -> Args {
    let mut arguments = env::args().skip(1);
    let mut path = env::current_dir().unwrap();
    let mut human_readable = false;
    let mut depth = None;
    let mut summarize = false;
    let mut bytes = false;
    let mut total = false;
    let mut block_size = None;
    let mut threshold = None;
    let mut x = None;
    let mut xclude = None;
    let mut a = false;

    while let Some(arg) = arguments.next() {
        match arg.as_str() {
            "--help" => print_help(),
            "-h" | "--human-readable" => human_readable = true,
            "-a" | "--all" => a = true,
            "-ah" => {
                a = true;
                human_readable = true;
            }
            "-sh" => {
                summarize = true;
                human_readable = true;
            }

            "-b" => bytes = true,
            "-s" | "--summarize" => summarize = true,
            "-c" | "--total" => total = true,
            "-d" | "--max-depth" => {
                depth = arguments.next().and_then(|v| v.parse().ok());
            }
            _ if arg.starts_with("-B") => {
                block_size = arg.strip_prefix("-B").map(String::from);
            }
            "-t" | "--threshold" => {
                threshold = arguments.next().and_then(|v| v.parse().ok());
            }
            "-x" | "--one-file-system" => {
                x = arguments.next().map(PathBuf::from);
            }
            "-X" | "--exclude-from" => {
                xclude = arguments.next().map(PathBuf::from);
            }
            _ => {
                if arg.starts_with('-') {
                    eprintln!("Error: Invalid argument '{}'", arg);
                    exit(1);
                }
                path = PathBuf::from(arg);
            }
        }
    }

    Args {
        depth,
        path,
        human_readable,
        bytes,
        summarize,
        total,
        block_size,
        threshold,
        xclude,
        x,
        a,
    }
}
fn scan_directory_iter(root_dir: &Path, max_depth: i32) -> BTreeMap<PathBuf, Vec<PathBuf>> {
    let current_dir = env::current_dir().unwrap();
    let cd = current_dir == root_dir;

    let mut dir_stack = Vec::with_capacity(256);
    let mut dir_map = BTreeMap::new();

    let no_depth = max_depth == 0;
    dir_stack.push((root_dir.to_path_buf(), 0));

    let mut file_names = Vec::with_capacity(64);
    let mut sub_dirs = Vec::with_capacity(64);

    while let Some((d_path, depth)) = dir_stack.pop() {
        file_names.clear();
        sub_dirs.clear();

        let open_dir = match nix::dir::Dir::open(&d_path, OFlag::O_RDONLY, Mode::empty()) {
            Ok(dir) => dir,
            Err(e) => {
                eprintln!("Failed to open {:?}: {}", d_path, e);
                continue;
            }
        };

        for res in open_dir {
            let entry = match res {
                Ok(entry) => entry,
                Err(e) => {
                    eprintln!("Error reading directory {:?}: {}", d_path, e);
                    continue;
                }
            };

            let file_name_bytes = entry.file_name().to_bytes();
            if file_name_bytes == b"." || file_name_bytes == b".." {
                continue;
            }

            let full_path = d_path.join(Path::new(entry.file_name().to_str().unwrap_or("")));

            match entry.file_type() {
                Some(nix::dir::Type::Directory) => {
                    if no_depth || depth < max_depth {
                        sub_dirs.push((full_path.clone(), depth + 1));
                    }
                }
                Some(nix::dir::Type::File) => {
                    file_names.push(full_path);
                }
                _ => {}
            }
        }

        let dir_key = if cd {
            PathBuf::from("./").join(d_path.strip_prefix(root_dir).unwrap_or(&d_path))
        } else {
            root_dir.join(d_path.strip_prefix(root_dir).unwrap_or(&d_path))
        };
        dir_map.insert(dir_key, file_names.clone());
        //sub_dirs.reverse();
        dir_stack.extend(sub_dirs.drain(..));
    }

    dir_map
}
#[cfg(test)]
mod tests;
fn main() -> Result<()> {
    let g_args = handle_args();
    let base_dir = &g_args.path;
    let depth = g_args.depth.unwrap_or(0);
    let dir_map = scan_directory_iter(base_dir, depth);
    let print_total_size = |total_size| {
        let output = get_file_size(None, Some(total_size));
        if g_args.human_readable {
            println!("{:<10} .", output.formatted);
        } else {
            println!("{:<10} .", total_size);
        }
    };

    let total_dir_size = calculate_total_dir_size(
        &dir_map,
        g_args.human_readable,
        g_args.bytes,
        g_args.a,
        |l| {
            if !g_args.summarize && depth != -1 {
                println!("{}", l);
            }
        },
    );

    //print_total_size(total_dir_size);

    Ok(())
}
