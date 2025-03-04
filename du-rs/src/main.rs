#![allow(unused_imports)]
#![allow(dead_code)]
#![allow(unused_variables)]
use nix::fcntl::OFlag;
use nix::{sys::stat::Mode, *};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::exit;
use std::{env, error::Error, result};

type Result<T> = result::Result<T, Box<dyn Error>>;

struct FileSize {
    bytes: i64,
    formatted: String,
}

fn get_file_size_in_bytes(file_path: &PathBuf) -> i64 {
    //Returns file size in bytes
    if let Ok(res) = nix::sys::stat::stat(file_path) {
        res.st_size
    } else {
        0
    }
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

fn format_file_size(file_map: &BTreeMap<PathBuf, Vec<PathBuf>>, arg: &str) -> Result<String> {
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
    dir: &BTreeMap<PathBuf, Vec<i64>>,
    format: bool,
    mut output_fn: F,
) -> i64
where
    F: FnMut(&str),
{
    let mut total_size: i64 = 0;

    for (dir_path, file_sizes) in dir.iter() {
        let dir_total: i64 = file_sizes.iter().sum();
        total_size += dir_total;

        let output = if format {
            let file_size = get_file_size(None, Some(dir_total));
            format!("{:<10} {}", file_size.formatted, dir_path.display())
        } else {
            format!("{:<10} {}", dir_total, dir_path.display())
        };

        output_fn(&output);
    }

    total_size
}
fn print_help() {
    println!(
        "Usage: myprogram [OPTIONS] [PATH]
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
fn scan_directory_iter(root_dir: &Path, max_depth: i32) -> BTreeMap<PathBuf, Vec<i64>> {
    let current_dir = env::current_dir().unwrap();
    let cd = current_dir == root_dir;
    let mut dir_stack = Vec::new();
    let mut dir_map = BTreeMap::new();
    let no_depth = max_depth == 0;
    dir_stack.push((root_dir.to_path_buf(), 0));

    while let Some((d_path, depth)) = dir_stack.pop() {
        let open_dir = match nix::dir::Dir::open(&d_path, OFlag::O_RDONLY, Mode::empty()) {
            Ok(dir) => dir,
            Err(e) => {
                eprintln!("Failed to open {:?}: {}", d_path, e);
                continue;
            }
        };
        let mut file_sizes = Vec::new();
        let mut sub_dirs = Vec::new();
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
                            if no_depth || depth < max_depth {
                                sub_dirs.push((full_path.clone(), depth + 1));
                            }
                        }
                        Some(nix::dir::Type::File) => {
                            let size = match nix::sys::stat::stat(&full_path) {
                                Ok(meta) => meta.st_blocks * 512,
                                Err(_) => 0,
                            };
                            file_sizes.push(size);
                        }
                        _ => {}
                    }
                }
                Err(e) => eprintln!("Error reading directory {:?}: {}", d_path, e),
            }
        }
        let dir_key = if cd {
            PathBuf::from("./").join(d_path.strip_prefix(root_dir).unwrap_or(&d_path))
        } else {
            root_dir.join(d_path.strip_prefix(root_dir).unwrap_or(&d_path))
        };
        dir_map.insert(dir_key, file_sizes);
        sub_dirs.reverse();
        dir_stack.extend(sub_dirs);
    }
    dir_map
}

fn main() -> Result<()> {
    let g_args = handle_args();
    let base_dir = &g_args.path;
    let depth = g_args.depth.unwrap_or(0);
    let dir_map = scan_directory_iter(base_dir, depth);
    if !g_args.summarize {
        let total_size = calculate_total_dir_size(&dir_map, g_args.human_readable, |l| {
            if g_args.a || depth != -1 {
                println!("{}", l);
            }
        });

        let output = get_file_size(None, Some(total_size));
        if g_args.human_readable {
            println!("{:<10} ", output.formatted);
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
