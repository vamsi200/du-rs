#![allow(unused_imports)]
#![allow(dead_code)]
#![allow(unused_variables)]
use nix::fcntl::OFlag;
use nix::{sys::stat::Mode, *};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::env::args;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::{clone, str};
use std::{env, error::Error, result};

type MyResult<T> = result::Result<T, Box<dyn Error>>;

fn get_file_size_in_bytes(file_path: &PathBuf) -> i64 {
    //Returns file size in bytes
    if let Ok(res) = nix::sys::stat::stat(file_path) {
        res.st_size
    } else {
        0
    }
}
struct FileSize {
    bytes: i64,
    formatted: String,
}

fn get_file_size(file_path: Option<&Path>, bytes: Option<i64>) -> FileSize {
    let bytes = if let Some(f) = file_path {
        if let Ok(res) = nix::sys::stat::stat(f) {
            Some(res.st_blocks * 512)
        } else {
            None
        }
    } else {
        bytes
    };

    let bytes = bytes.unwrap_or(0);

    let formatted = if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1048576 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else if bytes < 1073741824 {
        format!("{:.1}M", bytes as f64 / 1048576.0)
    } else {
        format!("{:.1}G", bytes as f64 / 1073741824.0)
    };

    FileSize { bytes, formatted }
}

fn format_file_size(file_map: &BTreeMap<PathBuf, Vec<PathBuf>>, arg: &str) -> MyResult<String> {
    for (dir_path, _files) in file_map {
        let res = nix::sys::stat::stat(dir_path).map_err(|_| "Failed to get file size")?;
        let bytes = res.st_blocks * 512;

        let output = match arg {
            "BT" => format!("{}T", (bytes as f64 / 1_099_511_627_776.0).ceil()),
            "BP" => format!("{}P", (bytes as f64 / 1_125_899_906_842_624.0).ceil()),
            "BE" => format!("{}E", (bytes as f64 / 1_152_921_504_606_846_976.0).ceil()),
            "BZ" => format!(
                "{}Z",
                (bytes as f64 / 1_180_591_620_717_411_303_424.0).ceil()
            ),
            "BY" => format!(
                "{}Y",
                (bytes as f64 / 1_208_925_819_614_629_174_706_176.0).ceil()
            ),
            "BR" => format!(
                "{}R",
                (bytes as f64 / 1_237_940_039_285_380_274_899_124_224.0).ceil()
            ),
            "BQ" => format!(
                "{}Q",
                (bytes as f64 / 1_267_650_600_228_229_401_496_703_205_376.0).ceil()
            ),
            _ => return Err("-B Requires a valid argument".into()),
        };

        return Ok(output);
    }

    Err("No valid directories found".into())
}

fn count_files(dir: &Path) -> u64 {
    todo!()
}

fn calculate_total_dir_size<F>(
    dir: &BTreeMap<PathBuf, Vec<PathBuf>>,
    format: bool,
    mut output_fn: F,
) -> i64
where
    F: FnMut(&str),
{
    let mut summarize_size: i64 = 0;
    let current_dir = env::current_dir().unwrap();

    for (dir_path, files) in dir {
        let mut dir_total: i64 = 0;

        for file in files {
            let file_size = get_file_size(Some(file), None);
            let stripped_file = file.strip_prefix(&current_dir).unwrap_or(file);

            let output = if format {
                format!("{:<10} ./{}", file_size.formatted, stripped_file.display())
            } else {
                format!("{:<10} ./{}", file_size.bytes, stripped_file.display())
            };

            output_fn(&output);
            dir_total += file_size.bytes;
        }

        let total_dir_formatted = get_file_size(None, Some(dir_total));

        let output = if format {
            format!(
                "{:<10} {}",
                total_dir_formatted.formatted,
                dir_path.display()
            )
        } else {
            format!("{:<10} {}", dir_total, dir_path.display())
        };

        output_fn(&output);
        summarize_size += dir_total;
    }

    summarize_size
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
            "-h" | "--human_readable" => human_readable = true,
            "-sh" => {
                human_readable = true;
                summarize = true;
            }
            "-a" | "-all" => a = true,
            "-ah" => {
                a = true;
                human_readable = true;
            }
            "-b" => bytes = true,
            "-s" | "--summarize" => summarize = true,
            "-c" | "--total" => total = true,
            "-d" | "--max-depth" => {
                if let Some(val) = arguments.next() {
                    depth = val.parse::<i32>().ok();
                }
            }
            _ if arg.starts_with("-B") => {
                block_size = arg.strip_prefix("-").map(|s| s.to_string());
            }
            "-t" | "--threshold" => {
                if let Some(val) = arguments.next() {
                    threshold = val.parse::<u64>().ok();
                }
            }
            "-x" | "--one-file-system" => {
                if let Some(val) = arguments.next() {
                    x = val.parse::<PathBuf>().ok();
                }
            }
            "-X" | "--exclude-from" => {
                if let Some(val) = arguments.next() {
                    xclude = val.parse::<PathBuf>().ok();
                }
            }
            _ => path = env::current_dir().unwrap(),
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
    let mut dir_stack = VecDeque::new();
    let mut dir_map = BTreeMap::new();
    let no_depth = max_depth == 0;
    dir_stack.push_back((root_dir.to_path_buf(), 0));

    while let Some((d_path, depth)) = dir_stack.pop_front() {
        let open_dir = match nix::dir::Dir::open(&d_path, OFlag::O_RDONLY, Mode::empty()) {
            Ok(dir) => dir,
            Err(e) => {
                eprintln!("Failed to open {:?}: {}", d_path, e);
                continue;
            }
        };

        let mut files = Vec::new();
        for res in open_dir {
            match res {
                Ok(entry) => {
                    let file_name = entry.file_name().to_string_lossy();
                    if file_name == "." || file_name == ".." {
                        continue;
                    }
                    let full_path = d_path.join(&*file_name);
                    match entry.file_type() {
                        Some(nix::dir::Type::Directory) => {
                            //when zero given f the checks..and proceed to push everything, ie.
                            //no_depth becomes true and so the other condition.
                            //if some value given then no_depth becomes false and now it depends on
                            //other check.. so it will push it until depth < max_depth.
                            if no_depth || depth < max_depth {
                                dir_stack.push_back((full_path.clone(), depth + 1));
                            }
                            let relative_dir = full_path
                                .strip_prefix(&current_dir)
                                .unwrap_or(&full_path)
                                .to_path_buf();
                            dir_map.insert(PathBuf::from("./").join(relative_dir), Vec::new());
                        }
                        Some(nix::dir::Type::File) => {
                            files.push(full_path);
                        }
                        _ => {}
                    }
                }
                Err(e) => eprintln!("Error reading directory {:?}: {}", d_path, e),
            }
        }

        let relative_d_path = d_path
            .strip_prefix(&current_dir)
            .unwrap_or(&d_path)
            .to_path_buf();
        let dir_key = PathBuf::from("./").join(relative_d_path);
        dir_map.insert(dir_key, files);
    }
    dir_map
}
fn main() -> MyResult<()> {
    let g_args = handle_args();
    let base_dir = &g_args.path;
    let depth = g_args.depth.unwrap_or(0);
    let dir_map = scan_directory_iter(base_dir, depth);

    // If summarize is set, we skip printing individual file sizes
    if !g_args.summarize {
        let total_size = calculate_total_dir_size(&dir_map, g_args.human_readable, |l| {
            if g_args.a || depth != -1 {
                println!("{}", l);
            }
        });

        let output = get_file_size(None, Some(total_size));
        if g_args.human_readable {
            println!("{:<10} .", output.formatted);
        } else {
            println!("{:<10} .", total_size);
        }
    } else {
        let total_size = calculate_total_dir_size(&dir_map, g_args.human_readable, |_| {});

        let output = get_file_size(None, Some(total_size));
        if g_args.human_readable {
            println!("{:<10} .", output.formatted);
        } else {
            println!("{:<10} .", total_size);
        }
    }

    Ok(())
}
