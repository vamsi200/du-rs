use crate::stat::lstat;
use fxhash::FxHashMap;
use fxhash::FxHashSet;
use nix::fcntl::OFlag;
use nix::sys::stat;
use nix::sys::stat::Mode;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fmt::Write;
use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::process::exit;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct FileStats {
    size: i64,
    blocks: i64,
}

impl FileStats {
    fn from(path: &PathBuf) -> Self {
        match stat::stat(path) {
            Ok(res) => Self {
                size: res.st_size,
                blocks: res.st_blocks,
            },
            Err(_) => Self { size: 0, blocks: 0 },
        }
    }

    fn size_in_bytes(&self) -> i64 {
        self.size
    }

    fn disk_usage_blocks(&self) -> i64 {
        (self.blocks * 512) / 1024
    }

    fn disk_usage_bytes(&self) -> i64 {
        self.blocks * 512
    }
}
const UNITS: [(&str, f64); 7] = [
    ("K", 1_024.0),
    ("M", 1_048_576.0),
    ("G", 1_073_741_824.0),
    ("T", 1_099_511_627_776.0),
    ("P", 1_125_899_906_842_624.0),
    ("E", 1_152_921_504_606_846_976.0),
    ("Z", 1_180_591_620_717_411_303_424.0),
];
fn parse_size_to_bytes(size_str: &str) -> Option<i64> {
    let size_str = size_str.trim().to_uppercase();
    let num_end = size_str
        .chars()
        .position(|c| !c.is_ascii_digit() && c != '.')
        .unwrap_or(size_str.len());

    let num_part: f64 = size_str[..num_end].parse().ok()?;
    let unit_part = &size_str[num_end..];

    let unit_map: HashMap<&str, f64> = UNITS.iter().cloned().collect();

    let multiplier = unit_map.get(unit_part).copied().unwrap_or(1.0);
    Some((num_part * multiplier) as i64)
}

fn get_file_sizes(file_path: Option<&Path>, bytes: Option<i64>) -> String {
    let bytes = if let Some(path) = file_path {
        stat::stat(path).map(|res| res.st_blocks * 512).unwrap_or(0)
    } else {
        bytes.unwrap_or(0)
    };

    if bytes < 1024 {
        return format!("{bytes}B");
    }

    let mut value = bytes as f64;
    let mut unit = "B";

    for (u, div) in UNITS.iter() {
        if bytes < (*div as i64) * 1024 {
            unit = u;
            value /= div;
            break;
        }
    }

    format!("{:.1}{}", value, unit)
}
//use Cow
//use fxhash
fn format_file_size<F>(
    dir: &BTreeMap<PathBuf, Vec<PathBuf>>,
    arg: &str,
    show_all: bool,
    threshold: String,
    mut output_fn: F,
) -> Result<String>
where
    F: FnMut(&str),
{
    let threshold_value = parse_size_to_bytes(&threshold).unwrap_or(0);
    let mut output_buffer = String::with_capacity(256);
    let c_dir =
        env::current_dir().map_err(|e| format!("Failed to get current directory: {}", e))?;

    let arg_str = arg;
    let arg_from_2 = &arg[2..];
    let format_size = |size: i64| -> Result<String> {
        if let Some((_, divisor)) = UNITS.iter().find(|&&(u, _)| arg_str == format!("-B{}", u)) {
            // If using unit-based sizes (-BG, -BM, etc.), ensure
            let adjusted_size = (size as f64 / divisor).ceil() as i64;
            return Ok(format!("{}{}", adjusted_size, arg_from_2));
        }

        if let Ok(block_size) = arg_from_2.parse::<i64>() {
            // For -B<size> (e.g., -B1024), use normal rounding behavior
            let adjusted_size = (size as f64 / block_size as f64).ceil() as i64;
            return Ok(adjusted_size.to_string());
        }

        Err("-B requires a valid argument".into())
    };

    let mut dir_sizes = BTreeMap::new();
    let mut total_size = 0;
    let get_size: fn(&FileStats) -> i64 = FileStats::disk_usage_bytes;
    for (dir_path, files) in dir.iter() {
        let dir_stats = FileStats::from(dir_path);
        let dir_size = get_size(&dir_stats);
        total_size += dir_size;
        dir_sizes.insert(dir_path.clone(), dir_size);

        for file in files {
            let file_stats = FileStats::from(file);
            let file_size = get_size(&file_stats);
            // updating the size of the directory in dir_sizes map by adding the size of the
            // current file
            dir_sizes
                .entry(dir_path.clone())
                .and_modify(|s| *s += file_size);

            total_size += file_size;

            if show_all && file_size >= threshold_value {
                output_buffer.clear();
                let formatted_file_size = format_size(file_size)?;
                let relative_path = file.strip_prefix(&c_dir).unwrap_or(file);
                write!(
                    output_buffer,
                    "{:<10} ./{}",
                    formatted_file_size,
                    relative_path.display()
                )
                .map_err(|e| format!("Failed to write to buffer: {}", e))?;
                output_fn(&output_buffer);
            }
        }
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
    for (dir_path, &dir_size) in dir_sizes.iter().rev() {
        let dir_relative_path = dir_path.strip_prefix(&c_dir).unwrap_or(dir_path);

        if dir_relative_path != Path::new(".") && dir_size >= threshold_value {
            output_buffer.clear();
            let formatted_dir_size = format_size(dir_size)?;
            write!(
                output_buffer,
                "{:<10} {}",
                formatted_dir_size,
                dir_relative_path.display()
            )
            .map_err(|e| format!("Failed to write to buffer: {}", e))?;
            output_fn(&output_buffer);
        }
    }

    output_buffer.clear();
    let formatted_total = format_size(total_size)?;
    write!(output_buffer, "{:<10} ./", formatted_total)
        .map_err(|e| format!("Failed to write to buffer: {}", e))?;
    output_fn(&output_buffer);

    Ok(formatted_total)
}

const fn select_dir_size_fn(is_bytes: bool, format: bool) -> fn(&FileStats) -> i64 {
    match (is_bytes, format) {
        (true, _) => |_| 0,
        (false, true) => FileStats::disk_usage_bytes,
        (false, false) => FileStats::disk_usage_blocks,
    }
}

const fn select_file_size_fn(is_bytes: bool, format: bool) -> fn(&FileStats) -> i64 {
    match (is_bytes, format) {
        (true, _) => FileStats::size_in_bytes,
        (false, true) => FileStats::disk_usage_bytes,
        (false, false) => FileStats::disk_usage_blocks,
    }
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
    F: std::io::Write,
{
    let get_dir_size = select_dir_size_fn(is_bytes, format);
    let get_file_size = select_file_size_fn(is_bytes, format);

    let c_dir = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut dir_sizes: FxHashMap<&Path, i64> = FxHashMap::default();
    let mut counted_inodes = rustc_hash::FxHashSet::default();
    let mut total_size = 0;

    let mut threshold_value = parse_size_to_bytes(&threshold).unwrap_or(0);
    if !is_bytes && !format {
        threshold_value /= 1024;
    }

    for (dir_path, file_names) in dir {
        let mut dir_size = get_dir_size(&FileStats::from(dir_path));
        total_size += dir_size;

        for file in file_names {
            let file_stats = FileStats::from(file);
            let file_size = get_file_size(&file_stats);

            if let Ok(metadata) = lstat(file) {
                let inode = metadata.st_ino;
                if l_arg || counted_inodes.insert(inode) {
                    total_size += file_size;
                }
            }
            dir_size += file_size;

            if r_files && file_size >= threshold_value {
                let relative_path = file.strip_prefix(&c_dir).unwrap_or(file);
                let display_path: Cow<str> = if file.starts_with(&c_dir) {
                    let mut s = String::from("./");
                    s.push_str(relative_path.to_string_lossy().as_ref());
                    Cow::Owned(s)
                } else {
                    Cow::Borrowed(relative_path.to_str().unwrap_or(""))
                };

                let mut formatted_size = String::new();
                if format {
                    formatted_size = get_file_sizes(None, Some(file_size));
                } else {
                    write!(&mut formatted_size, "{}", file_size).unwrap();
                }

                writeln!(output_fn, "{:<10} {}", formatted_size, display_path).unwrap();
            }
        }

        dir_sizes.insert(dir_path, dir_size);
    }
    let mut sorted_dirs: Vec<_> = dir_sizes.keys().copied().collect();
    sorted_dirs.sort_unstable_by(|a, b| b.cmp(a));

    for &dir_path in &sorted_dirs {
        if let Some(parent) = dir_path.parent() {
            let dir_size = *dir_sizes.get(dir_path).unwrap_or(&0);
            *dir_sizes.entry(parent).or_insert(0) += dir_size;
        }
    }

    for &dir_path in &sorted_dirs {
        let dir_size = *dir_sizes.get(dir_path).unwrap_or(&0);
        if dir_size < threshold_value {
            continue;
        }

        let relative_path = match dir_path.strip_prefix(&c_dir) {
            Ok(rel) => Cow::Owned(rel.to_string_lossy().into_owned()),
            Err(_) => Cow::Borrowed(dir_path.to_str().unwrap_or("")),
        };

        let formatted_size = if format {
            get_file_sizes(None, Some(dir_size))
        } else {
            dir_size.to_string()
        };

        writeln!(output_fn, "{:<10} {}", formatted_size, relative_path).unwrap();
        output_fn.flush().unwrap();
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
            } else if let Some(stripped) = trimmed_line.strip_prefix("*.") {
                let extension = stripped;
                hs.insert(FileContent::Pattern(extension.to_string()));
            }
        } else {
            let full_path = current_dir.join(path);
            if full_path.exists() && full_path.is_dir() {
                hs.insert(FileContent::Path(full_path));
            } else if let Some(stripped) = trimmed_line.strip_prefix("*.") {
                let extension = stripped;
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
    use nix::dir::Dir;
    use nix::fcntl::OFlag;
    use nix::sys::stat::Mode;
    use std::collections::BTreeMap;
    use std::ffi::OsStr;

    let current_dir = env::current_dir()?;
    let cd = current_dir == root_dir;
    let mut dir_stack = Vec::with_capacity(256);
    let mut dir_map = BTreeMap::new();
    let mut visited = FxHashSet::default();

    let root_dev = if x_option.is_some() {
        Some(nix::sys::stat::stat(root_dir)?.st_dev)
    } else {
        None
    };

    let no_depth = max_depth == 0;

    let mut exclusion_paths = FxHashSet::default();
    let mut exclusion_patterns = FxHashSet::default();
    if let Some(exclude_path) = is_exclude {
        for s in exclude_list(exclude_path) {
            match s {
                FileContent::Path(p) => exclusion_paths.insert(p),
                FileContent::Pattern(pt) => {
                    exclusion_patterns.insert(OsStr::new(&pt).to_os_string())
                }
            };
        }
    }
    let initial_dir_key = if cd {
        PathBuf::from("./")
    } else {
        root_dir.to_path_buf()
    };
    dir_stack.push((root_dir.to_path_buf(), initial_dir_key, 0));

    while let Some((absolute_path, dir_key, depth)) = dir_stack.pop() {
        if let Some(root_dev) = root_dev {
            let sub_dir_dev = nix::sys::stat::stat(&absolute_path)?.st_dev;
            if root_dev != sub_dir_dev {
                continue;
            }
        }

        let mut file_names = Vec::new();
        let mut subdirs = Vec::new();

        let open_dir = Dir::open(&absolute_path, OFlag::O_RDONLY, Mode::empty())
            .map_err(|e| format!("Failed to open {:?}: {}", absolute_path, e))?;

        for entry in open_dir {
            let entry =
                entry.map_err(|e| format!("Error reading directory {:?}: {}", absolute_path, e))?;
            let file_name = entry.file_name();
            let file_name_str = file_name.to_str().unwrap_or("");
            if file_name_str == "." || file_name_str == ".." {
                continue;
            }

            let full_path = absolute_path.join(file_name_str);

            if exclusion_paths.contains(&full_path) {
                continue;
            }
            if let Some(ext) = full_path.extension() {
                if exclusion_patterns.contains(ext) {
                    continue;
                }
            }

            match entry.file_type() {
                Some(nix::dir::Type::Directory) => {
                    if !no_depth && depth >= max_depth {
                        continue;
                    }
                    if visited.insert(full_path.to_owned()) {
                        let mut new_dir_key = dir_key.to_owned();
                        new_dir_key.push(file_name_str);
                        subdirs.push((full_path, new_dir_key, depth + 1));
                    }
                }
                Some(nix::dir::Type::File) => {
                    file_names.push(full_path);
                }
                _ => {}
            }
        }

        for (abs_path, key, depth) in subdirs.into_iter().rev() {
            dir_stack.push((abs_path, key, depth));
        }

        dir_map.insert(dir_key, file_names);
    }

    Ok(dir_map)
}
#[cfg(test)]
mod tests;
fn main() -> Result<()> {
    let g_args = handle_args();
    let mut output_size = std::io::stdout().lock();
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
            &mut output_size,
        );

        let output = get_file_sizes(None, Some(total_dir_size));

        if g_args.summarize {
            if g_args.human_readable {
                println!("{:<10}  .", output);
            } else {
                println!("{:<10} .", total_dir_size);
            }
        } else if g_args.total {
            if g_args.human_readable {
                println!("{:<10} total", output);
            } else {
                println!("{:<10} total", total_dir_size);
            }
        }
    }

    Ok(())
}
