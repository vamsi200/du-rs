use nix::fcntl::OFlag;
use nix::sys::stat::Mode;
use std::collections::{BTreeMap, HashSet};
use std::env;
use std::ffi::OsStr;
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::process::exit;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const UNITS: [(&str, f64); 7] = [
    ("K", 1_024.0),
    ("M", 1_048_576.0),
    ("G", 1_073_741_824.0),
    ("T", 1_099_511_627_776.0),
    ("P", 1_125_899_906_842_624.0),
    ("E", 1_152_921_504_606_846_976.0),
    ("Z", 1_180_591_620_717_411_303_424.0),
];

fn get_file_size_in_bytes(file_path: &PathBuf) -> i64 {
    match nix::sys::stat::stat(file_path) {
        Ok(res) => res.st_size,
        Err(_) => 0,
    }
}

fn get_disk_usage_blocks(path: &PathBuf) -> i64 {
    match nix::sys::stat::stat(path) {
        Ok(res) => (res.st_blocks * 512) / 1024,
        Err(_) => 0,
    }
}

fn get_disk_usage_bytes(path: &PathBuf) -> i64 {
    match nix::sys::stat::stat(path) {
        Ok(res) => res.st_blocks * 512,
        Err(_) => 0,
    }
}

fn parse_size_to_bytes(size_str: &str) -> Option<i64> {
    let size_str = size_str.trim().to_uppercase();
    let num_end = size_str
        .chars()
        .position(|c| !c.is_digit(10) && c != '.')
        .unwrap_or(size_str.len());
    let num_part: f64 = size_str[..num_end].parse().ok()?;
    let unit_part = &size_str[num_end..];

    let multiplier: i64 = match unit_part {
        "B" | "" => 1,
        "K" | "KB" => 1024,
        "M" | "MB" => 1024 * 1024,
        "G" | "GB" => 1024 * 1024 * 1024,
        "T" | "TB" => 1024 * 1024 * 1024 * 1024,
        _ => return None,
    };

    Some((num_part * multiplier as f64) as i64)
}

struct FileSize {
    bytes: i64,
    formatted: String,
}

fn get_file_size(file_path: Option<&Path>, bytes: Option<i64>) -> FileSize {
    static UNITS: [&str; 3] = ["K", "M", "G"];
    static DIVISORS: [i64; 3] = [1024, 1048576, 1073741824];

    let bytes = if let Some(f) = file_path {
        match nix::sys::stat::stat(f) {
            Ok(res) => res.st_blocks * 512,
            Err(_) => 0,
        }
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

fn format_file_size<F>(
    dir: &BTreeMap<PathBuf, Vec<PathBuf>>,
    arg: &String,
    show_all: bool,
    threshold: String,
    mut output_fn: F,
) -> Result<String>
where
    F: FnMut(&str),
{
    let threshold_value = parse_size_to_bytes(threshold.as_str()).unwrap_or(0);
    let mut output_buffer = String::with_capacity(256);
    let c_dir =
        env::current_dir().map_err(|e| format!("Failed to get current directory: {}", e))?;

    let arg_str = arg.as_str();
    let arg_from_2 = arg
        .get(2..)
        .ok_or_else(|| "-B argument is invalid".to_string())?;

    let format_size = |size: i64| -> Result<String> {
        if let Some((_, divisor)) = UNITS.iter().find(|&&(u, _)| arg_str == format!("-B{}", u)) {
            return Ok(format!(
                "{}{}",
                (size as f64 / divisor).ceil() as i64,
                arg_from_2
            ));
        }

        let block_size = arg_from_2
            .parse::<i64>()
            .map_err(|_| "-B requires a valid argument".to_string())?;
        let adjusted_size = ((size as f64 / block_size as f64).ceil() * block_size as f64) as i64;
        Ok(adjusted_size.to_string())
    };

    let mut empty_file = String::new();
    let mut dir_sizes = BTreeMap::new();
    let mut total_size = 0;

    for (dir_path, files) in dir.iter() {
        let mut dir_size = get_disk_usage_bytes(dir_path);
        total_size += dir_size;

        for file in files {
            let file_size = get_disk_usage_bytes(file);

            if file_size == 0 {
                if let Ok(rel_path) = file.strip_prefix(&c_dir) {
                    if let Some(first_part) = rel_path.to_str().and_then(|s| s.split('/').next()) {
                        empty_file = first_part.to_string();
                    }
                }
            }

            dir_size += file_size;
            total_size += file_size;

            if show_all && file_size >= threshold_value {
                output_buffer.clear();
                let formatted_file_size = format_size(file_size)?;
                let relative_path = file.strip_prefix(&c_dir).unwrap_or(file);
                output_buffer.push_str(&format!(
                    "{:<10} ./{}",
                    formatted_file_size,
                    relative_path.display()
                ));
                output_fn(&output_buffer);
            }
        }

        dir_sizes.insert(dir_path.clone(), dir_size);
    }

    for (dir_path, &dir_size) in dir_sizes.clone().iter() {
        let mut current_path = dir_path.clone();
        while let Some(parent) = current_path.parent() {
            if let Some(parent_size) = dir_sizes.get_mut(&parent.to_path_buf()) {
                *parent_size += dir_size;
            }
            current_path = parent.to_path_buf();
        }
    }

    for (dir_path, &dir_size) in dir_sizes.iter().rev() {
        let dir_relative_path = dir_path.strip_prefix(&c_dir).unwrap_or(dir_path);
        let mut display_size = dir_size;

        let root_dir = dir_relative_path
            .to_str()
            .map(|s| s.trim_end_matches('/').trim_start_matches("./"))
            .unwrap_or("");

        if root_dir == empty_file {
            display_size += 4096;
        }

        if dir_relative_path != PathBuf::from(".") && display_size >= threshold_value {
            let formatted_dir_size = format_size(display_size)?;
            output_buffer.clear();
            output_buffer.push_str(&format!(
                "{:<10} {}",
                formatted_dir_size,
                dir_relative_path.display()
            ));
            output_fn(&output_buffer);
        }
    }

    let formatted_total_dir_size = format_size(total_size)?;
    output_buffer.clear();
    output_buffer.push_str(&format!("{:<10} ./", formatted_total_dir_size));
    output_fn(&output_buffer);

    Ok(formatted_total_dir_size)
}
fn calculate_total_dir_size<F>(
    dir: &BTreeMap<PathBuf, Vec<PathBuf>>,
    format: bool,
    is_bytes: bool,
    r_files: bool,
    threshold: String,
    l_arg: bool,
    mut output_fn: F,
) -> i64
where
    F: FnMut(&str),
{
    let c_dir = match env::current_dir() {
        Ok(dir) => dir,
        Err(_) => {
            eprintln!("Error: Failed to get current directory");
            return 0;
        }
    };

    let mut dir_sizes = BTreeMap::new();
    let mut output_buffer = String::with_capacity(256);
    let mut empty_file = String::new();
    let mut total_size = 0;
    let mut counted_inodes = HashSet::new();

    let threshold_value = if is_bytes || format {
        parse_size_to_bytes(&threshold).unwrap_or(0)
    } else {
        parse_size_to_bytes(&threshold).unwrap_or(0) / 1024
    };

    for (dir_path, file_names) in dir.iter() {
        let mut dir_size = if is_bytes {
            0
        } else if format {
            get_disk_usage_bytes(dir_path)
        } else {
            get_disk_usage_blocks(dir_path)
        };
        total_size += dir_size;

        for file in file_names {
            let file_size = if !is_bytes && !format {
                get_disk_usage_blocks(file)
            } else if is_bytes {
                get_file_size_in_bytes(file)
            } else {
                get_disk_usage_bytes(file)
            };

            if file_size == 0 {
                if let Ok(rel_path) = file.strip_prefix(&c_dir) {
                    if let Some(first_part) = rel_path.to_str().and_then(|s| s.split('/').next()) {
                        empty_file = first_part.to_string();
                    }
                }
            }

            if let Ok(metadata) = nix::sys::stat::lstat(file) {
                let inode = metadata.st_ino;
                if l_arg || counted_inodes.insert(inode) {
                    total_size += file_size;
                }
            }
            dir_size += file_size;

            if r_files && file_size >= threshold_value {
                output_buffer.clear();

                let relative_path = file.strip_prefix(&c_dir).unwrap_or(file);
                let display_path = if file.starts_with(&c_dir) {
                    format!("./{}", relative_path.display())
                } else {
                    relative_path.display().to_string()
                };

                let formatted_output = if format {
                    let formatted_size = get_file_size(None, Some(file_size));
                    format!("{:<10} {}", formatted_size.formatted, display_path)
                } else {
                    format!("{:<10} {}", file_size, display_path)
                };

                output_fn(&formatted_output);
            }
        }

        dir_sizes.insert(dir_path.clone(), dir_size);
    }

    for (dir_path, &dir_size) in dir_sizes.clone().iter() {
        let mut current_path = dir_path.clone();
        while let Some(parent) = current_path.parent() {
            if let Some(parent_size) = dir_sizes.get_mut(parent) {
                *parent_size += dir_size;
            }
            current_path = parent.to_path_buf();
        }
    }

    for (i, (&ref dir_path, &dir_size)) in dir_sizes.iter().rev().enumerate() {
        output_buffer.clear();
        let relative_path = dir_path.to_str().unwrap_or("").trim_end_matches('/');
        let root_dir = relative_path.trim_start_matches("./");

        let display_size = if root_dir == empty_file {
            if is_bytes {
                dir_size
            } else if format {
                dir_size + 4096
            } else {
                dir_size + 4
            }
        } else {
            dir_size
        };

        let formatted_output = if i == dir_sizes.len() - 1 {
            if format {
                let formatted_size = get_file_size(None, Some(total_size));
                if dir_size >= threshold_value {
                    format!("{:<10} {}", formatted_size.formatted, relative_path)
                } else {
                    String::new()
                }
            } else {
                if dir_size >= threshold_value {
                    format!("{:<10} {}", total_size, relative_path)
                } else {
                    String::new()
                }
            }
        } else {
            if format {
                let formatted_size = get_file_size(None, Some(display_size));
                if dir_size >= threshold_value {
                    format!("{:<10} {}", formatted_size.formatted, relative_path)
                } else {
                    String::new()
                }
            } else {
                if dir_size >= threshold_value {
                    format!("{:<10} {}", display_size, relative_path)
                } else {
                    String::new()
                }
            }
        };

        if !formatted_output.is_empty() {
            output_fn(&formatted_output);
        }
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
    block_size: String,
    threshold: Option<String>,
    x: Option<PathBuf>,
    xclude: Option<PathBuf>,
    a: bool,
    l: bool,
}

fn handle_args() -> Args {
    let mut arguments = env::args().skip(1);
    let mut path = env::current_dir().unwrap();
    let mut human_readable = false;
    let mut depth = None;
    let mut summarize = false;
    let mut bytes = false;
    let mut total = false;
    let mut block_size = String::new();
    let mut threshold = None;
    let mut x = None;
    let mut xclude = None;
    let mut a = false;
    let mut l = false;
    while let Some(arg) = arguments.next() {
        match arg.as_str() {
            "--help" => print_help(),
            "-h" | "--human-readable" => human_readable = true,
            "-a" | "--all" => a = true,
            "-l" => l = true,
            "-ah" => {
                a = true;
                human_readable = true;
            }
            "-c" | "--total" => total = true,
            "-sh" => {
                summarize = true;
                human_readable = true;
            }
            "-b" => bytes = true,
            "-s" | "--summarize" => summarize = true,
            "-d" | "--max-depth" => {
                depth = arguments.next().and_then(|v| v.parse().ok());
            }
            _ if arg.starts_with("-B") => {
                block_size = arg;
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
        l,
    }
}
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum FileContent {
    Path(PathBuf),
    Pattern(String),
}

fn exclude_list(file: &Path) -> HashSet<FileContent> {
    let file_fd: RawFd = nix::fcntl::open(file, OFlag::O_RDONLY, Mode::empty()).unwrap();
    let mut buffer = [0u8; 1024];
    let mut content = String::new();
    let mut hs = HashSet::new();

    loop {
        let bytes_read = nix::unistd::read(file_fd, &mut buffer).unwrap();
        if bytes_read == 0 {
            break;
        }
        content.push_str(&String::from_utf8_lossy(&buffer[..bytes_read]));
    }
    nix::unistd::close(file_fd).unwrap();

    let current_dir = env::current_dir().unwrap();

    for line in content.lines() {
        let trimmed_line = line.trim();

        if trimmed_line.is_empty() {
            continue;
        }

        let path = Path::new(trimmed_line);
        if path.is_absolute() {
            if path.exists() && path.is_dir() {
                hs.insert(FileContent::Path(path.to_path_buf()));
            } else if trimmed_line.starts_with("*.") {
                let extension = &trimmed_line[2..];
                hs.insert(FileContent::Pattern(extension.to_string()));
            }
        } else {
            let full_path = current_dir.join(path);
            if full_path.exists() && full_path.is_dir() {
                hs.insert(FileContent::Path(full_path));
            } else if trimmed_line.starts_with("*.") {
                let extension = &trimmed_line[2..];
                hs.insert(FileContent::Pattern(extension.to_string()));
            }
        }
    }
    hs
}
fn scan_directory_iter(
    root_dir: &Path,
    max_depth: i32,
    x_option: Option<&Path>,
    is_exclude: Option<&Path>,
) -> Result<BTreeMap<PathBuf, Vec<PathBuf>>> {
    let current_dir = env::current_dir()?;
    let cd = current_dir == root_dir;
    let mut dir_stack = Vec::with_capacity(256);
    let mut dir_map = BTreeMap::new();

    let root_dev = if let Some(_) = x_option {
        Some(nix::sys::stat::stat(root_dir)?.st_dev)
    } else {
        None
    };

    let no_depth = max_depth == 0;
    dir_stack.push((root_dir.to_path_buf(), 0));

    let mut file_names = Vec::with_capacity(64);
    let mut sub_dirs = Vec::with_capacity(64);
    let mut exclusion_paths = Vec::new();
    let mut exclusion_patterns = Vec::new();

    if let Some(exclude_path) = is_exclude {
        for s in exclude_list(exclude_path) {
            match s {
                FileContent::Path(p) => exclusion_paths.push(p),
                FileContent::Pattern(pt) => exclusion_patterns.push(OsStr::new(&pt).to_os_string()),
            }
        }
    }

    while let Some((d_path, depth)) = dir_stack.pop() {
        file_names.clear();
        sub_dirs.clear();

        if let Some(root_dev) = root_dev {
            let sub_dir_dev = nix::sys::stat::stat(&d_path)?.st_dev;
            if root_dev != sub_dir_dev {
                continue;
            }
        }

        let open_dir = nix::dir::Dir::open(&d_path, OFlag::O_RDONLY, Mode::empty())
            .map_err(|e| format!("Failed to open {:?}: {}", d_path, e))?;

        for res in open_dir {
            let entry = res.map_err(|e| format!("Error reading directory {:?}: {}", d_path, e))?;

            let file_name = entry.file_name().to_str().unwrap_or("");
            if file_name == "." || file_name == ".." {
                continue;
            }

            let full_path = d_path.join(file_name);

            if exclusion_paths.iter().any(|ex| full_path == *ex) {
                continue;
            }
            if let Some(ext) = full_path.extension() {
                if exclusion_patterns.iter().any(|pat| ext == pat) {
                    continue;
                }
            }

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
        dir_stack.extend(sub_dirs.drain(..));
    }

    Ok(dir_map)
}
#[cfg(test)]
mod tests;
fn main() -> Result<()> {
    let g_args = handle_args();

    let base_dir = g_args.x.clone().unwrap_or_else(|| g_args.path.clone());
    let depth = g_args.depth.unwrap_or(0);

    let dir_map = scan_directory_iter(
        &base_dir,
        depth,
        g_args.x.as_deref(),
        g_args.xclude.as_deref(),
    )?;

    if !g_args.block_size.is_empty() {
        let output = format_file_size(
            &dir_map,
            &g_args.block_size,
            g_args.a,
            g_args.threshold.unwrap_or_default(),
            |l| {
                if !g_args.summarize && depth != -1 {
                    println!("{}", l);
                }
            },
        )?;

        if g_args.summarize {
            println!("{:<10}  .", output);
        }
    } else {
        let total_dir_size = calculate_total_dir_size(
            &dir_map,
            g_args.human_readable,
            g_args.bytes,
            g_args.a,
            g_args.threshold.unwrap_or_default(),
            g_args.l,
            |l| {
                if !g_args.summarize && depth != -1 {
                    println!("{}", l);
                }
            },
        );

        let output = get_file_size(None, Some(total_dir_size));

        if g_args.summarize {
            if g_args.human_readable {
                println!("{:<10}  .", output.formatted);
            } else {
                println!("{:<10} .", total_dir_size);
            }
        } else if g_args.total {
            if g_args.human_readable {
                println!("{:<10} total", output.formatted);
            } else {
                println!("{:<10} total", total_dir_size);
            }
        }
    }

    Ok(())
}
