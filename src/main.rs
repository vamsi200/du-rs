#![allow(unused_variables)]
#![allow(dead_code)]

use fxhash::FxHashSet;
use nix::{
    fcntl::OFlag,
    sys::stat::{self, Mode},
};
use std::io::{BufWriter, Write};
use std::{
    collections::{HashMap, HashSet},
    env,
    os::fd::RawFd,
    path::{Path, PathBuf},
    process::exit,
};

type Cresult<T> = anyhow::Result<T, anyhow::Error>;
use anyhow::{Context, Error, Result};
struct FileStats {
    size: i64,
    blocks: i64,
}

impl FileStats {
    fn from(path: &Path) -> Self {
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
    use std::fmt::Write;
    let bytes = bytes.unwrap_or_else(|| {
        file_path
            .and_then(|path| stat::stat(path).ok())
            .map_or(0, |res| res.st_blocks * 512)
    });

    let mut output = String::with_capacity(16);

    if bytes < 1024 {
        write!(output, "{}B", bytes).unwrap();
        return output;
    }

    let mut value = bytes as f64;
    let mut unit = "B";

    for &(u, div) in UNITS.iter() {
        if bytes < (div as i64) * 1024 {
            unit = u;
            value /= div;
            break;
        }
    }

    write!(output, "{:.1}{}", value, unit).unwrap();
    output
}
fn format_size(size: i64, arg: &str) -> Cresult<String> {
    let arg_from_2 = &arg[2..];

    if let Some((_, divisor)) = UNITS.iter().find(|&&(u, _)| arg == format!("-B{}", u)) {
        let adjusted_size = (size as f64 / divisor).ceil() as i64;

        return Ok(format!("{}{}", adjusted_size, arg_from_2));
    }

    if let Ok(block_size) = arg_from_2.parse::<i64>() {
        let adjusted_size = (size as f64 / block_size as f64).ceil() as i64;
        return Ok(adjusted_size.to_string());
    } else {
        return Err(Error::msg("-B requires a valid argument"));
    }
}
#[derive(Debug, Clone)]
enum SizeFormat {
    Bytes,
    HumanReadable,
    Blocks,
}

impl SizeFormat {
    fn get_dir_size(&self, stats: &FileStats) -> i64 {
        match self {
            SizeFormat::Bytes => 0,
            SizeFormat::HumanReadable => stats.disk_usage_bytes(),
            SizeFormat::Blocks => stats.disk_usage_blocks(),
        }
    }

    fn get_file_size(&self, stats: &FileStats) -> i64 {
        match self {
            SizeFormat::Bytes => stats.size_in_bytes(),
            SizeFormat::HumanReadable => stats.disk_usage_bytes(),
            SizeFormat::Blocks => stats.disk_usage_blocks(),
        }
    }
}

fn calculate_directory_sizes(
    dir_path: &Path,
    files: &[PathBuf],
    list_files: bool,
    summarize: bool,
    threshold_value: i64,
    c_dir: &Path,
    writer: &mut BufWriter<std::io::Stdout>,
    seen_inodes: &mut FxHashSet<(u64, u64)>,
    count_links: bool,
    arg: &str,
) -> Cresult<i64> {
    let size_format = SizeFormat::HumanReadable;

    let dir_stats = FileStats::from(dir_path);
    let initial_dir_size = size_format.get_dir_size(&dir_stats);
    let mut total_size = initial_dir_size;

    for file in files {
        let meta = match stat::stat(file) {
            Ok(m) => m,
            Err(_) => continue,
        };

        if !count_links && meta.st_nlink > 1 {
            let inode = (meta.st_dev as u64, meta.st_ino as u64);
            if !seen_inodes.insert(inode) {
                continue;
            }
        }

        let file_stats = FileStats {
            size: meta.st_size,
            blocks: meta.st_blocks,
        };
        let file_size = size_format.get_file_size(&file_stats);

        total_size += file_size;

        if list_files && !summarize && file_size >= threshold_value {
            let formatted_file_size = format_size(file_size, arg)?;
            let relative_path = file.strip_prefix(c_dir).unwrap_or(file);
            let display_path = if file.starts_with(c_dir) {
                format!("./{}", relative_path.to_string_lossy())
            } else {
                relative_path.to_string_lossy().into_owned()
            };

            writeln!(writer, "{:<10} {}", formatted_file_size, display_path).unwrap();
        }
    }

    if !summarize && total_size >= threshold_value {
        let formatted_dir_size = format_size(total_size, arg)?;
        let dir_relative_path = dir_path.strip_prefix(c_dir).unwrap_or(dir_path);

        if dir_relative_path != Path::new(".") {
            writeln!(
                writer,
                "{:<10} {}",
                formatted_dir_size,
                dir_relative_path.display()
            )
            .unwrap();
        }
    }

    Ok(total_size)
}

pub fn calculate_directory_size_default(
    dir_path: &Path,
    files: &[PathBuf],
    format: bool,
    is_bytes: bool,
    summarize: bool,
    list_files: bool,
    threshold_size: i64,
    count_links: bool,
    writer: &mut BufWriter<std::io::Stdout>,
    seen_inodes: &mut FxHashSet<(u64, u64)>,
    current_dir: &Path,
) -> i64 {
    let size_format = if is_bytes {
        SizeFormat::Bytes
    } else if format {
        SizeFormat::HumanReadable
    } else {
        SizeFormat::Blocks
    };

    let dir_stats = FileStats::from(&dir_path);
    let initial_dir_size = size_format.get_dir_size(&dir_stats);
    let mut total_size = initial_dir_size;

    for file in files {
        let meta = match stat::stat(file) {
            Ok(m) => m,
            Err(_) => continue,
        };

        if !count_links && meta.st_nlink > 1 {
            let inode = (meta.st_dev as u64, meta.st_ino as u64);
            if !seen_inodes.insert(inode) {
                continue;
            }
        }

        let file_stats = FileStats {
            size: meta.st_size,
            blocks: meta.st_blocks,
        };
        let file_size = size_format.get_file_size(&file_stats);

        total_size += file_size;

        if list_files && !summarize && file_size >= threshold_size {
            let relative_display = match file.strip_prefix(current_dir) {
                Ok(rel) => rel.to_string_lossy(),
                Err(_) => file.to_string_lossy(),
            };

            if format {
                let formatted_size = get_file_sizes(None, Some(file_size));
                if file.starts_with(current_dir) {
                    writeln!(writer, "{:<10} ./{}", formatted_size, relative_display).unwrap();
                } else {
                    writeln!(writer, "{:<10} {}", formatted_size, relative_display).unwrap();
                }
            } else {
                if file.starts_with(current_dir) {
                    writeln!(writer, "{:<10} ./{}", file_size, relative_display).unwrap();
                } else {
                    writeln!(writer, "{:<10} {}", file_size, relative_display).unwrap();
                }
            }
        }
    }

    if !summarize && total_size >= threshold_size {
        let relative_display = match dir_path.strip_prefix(current_dir) {
            Ok(rel) => rel.to_string_lossy(),
            Err(_) => dir_path.to_string_lossy(),
        };

        if format {
            let formatted_size = get_file_sizes(None, Some(total_size));
            if dir_path.starts_with(current_dir) {
                writeln!(writer, "{:<10} ./{}", formatted_size, relative_display).unwrap();
            } else {
                writeln!(writer, "{:<10} {}", formatted_size, relative_display).unwrap();
            }
        } else {
            if dir_path.starts_with(current_dir) {
                writeln!(writer, "{:<10} ./{}", total_size, relative_display).unwrap();
            } else {
                writeln!(writer, "{:<10} {}", total_size, relative_display).unwrap();
            }
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

#[derive(Debug, Clone)]
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
    c: bool,
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
    let mut c = false;
    while let Some(arg) = arguments.next() {
        match arg.as_str() {
            "--help" => print_help(),
            "-h" | "--human-readable" => human_readable = true,
            "-a" | "--all" => a = true,
            "-l" => l = true,
            "-c" | "--total" => {
                total = true;
                c = true;
            }
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
        c,
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
    count_links: bool,
    args: Args,
) -> Result<i64> {
    use fxhash::FxHashSet;
    use nix::dir::Dir;
    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::{fstat, stat, Mode};
    use std::env;
    use std::ffi::OsStr;
    use std::io::{stdout, BufWriter};
    use std::os::unix::ffi::OsStrExt;

    let current_dir = env::current_dir().context("Failed to get current directory")?;
    let cd = current_dir == root_dir;

    let mut dir_stack: Vec<(PathBuf, PathBuf, i32)> = Vec::with_capacity(256);
    let mut visited_dirs = FxHashSet::default();
    let mut seen_inodes: FxHashSet<(u64, u64)> = FxHashSet::default();
    let threshold = args.threshold.as_deref().unwrap_or("0");

    let threshold_value = parse_size_to_bytes(threshold).unwrap_or(0);
    let threshold_size = if args.bytes {
        threshold_value
    } else if args.human_readable {
        threshold_value
    } else {
        threshold_value / 1024
    };

    let root_dev = if x_option.is_some() {
        Some(
            stat(root_dir)
                .context("Failed to get device ID of root directory")?
                .st_dev,
        )
    } else {
        None
    };

    let no_depth = max_depth == 0;

    let mut exclusion_paths = FxHashSet::default();
    let mut exclusion_patterns = FxHashSet::default();
    if let Some(exclude_path) = is_exclude {
        for s in crate::exclude_list(exclude_path) {
            match s {
                FileContent::Path(p) => {
                    exclusion_paths.insert(p);
                }
                FileContent::Pattern(pt) => {
                    exclusion_patterns.insert(OsStr::new(&pt).to_os_string());
                }
            }
        }
    }
    let use_exclusion = is_exclude.is_some();

    let initial_dir_key = if cd {
        PathBuf::from("./")
    } else {
        root_dir.to_path_buf()
    };

    dir_stack.push((root_dir.to_path_buf(), initial_dir_key, 0));
    let mut writer = BufWriter::new(stdout());
    let mut total_size = 0;
    while let Some((absolute_path, dir_key, depth)) = dir_stack.pop() {
        let fd = match open(
            &absolute_path,
            OFlag::O_DIRECTORY | OFlag::O_RDONLY,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(_) => continue,
        };

        let dir_meta = match fstat(fd) {
            Ok(m) => m,
            Err(_) => {
                let _ = nix::unistd::close(fd);
                continue;
            }
        };

        if let Some(root_dev) = root_dev {
            if dir_meta.st_dev != root_dev {
                let _ = nix::unistd::close(fd);
                continue;
            }
        }

        let open_dir = match Dir::from_fd(fd) {
            Ok(d) => d,
            Err(_) => {
                let _ = nix::unistd::close(fd);
                continue;
            }
        };

        let mut file_names: Vec<PathBuf> = Vec::with_capacity(64);
        let mut subdirs: Vec<(PathBuf, PathBuf, i32)> = Vec::with_capacity(16);

        for entry_res in open_dir {
            let entry = match entry_res {
                Ok(e) => e,
                Err(_) => continue,
            };

            let name_bytes = entry.file_name().to_bytes();
            if name_bytes == b"." || name_bytes == b".." {
                continue;
            }

            let file_name_os_str = OsStr::from_bytes(name_bytes);
            let full_path = absolute_path.join(file_name_os_str);

            if use_exclusion {
                if exclusion_paths.contains(&full_path) {
                    continue;
                }
                if let Some(ext) = full_path.extension() {
                    if exclusion_patterns.contains(ext) {
                        continue;
                    }
                }
            }

            match entry.file_type() {
                Some(nix::dir::Type::Directory) => {
                    if !no_depth && depth >= max_depth {
                        continue;
                    }
                    if visited_dirs.insert(full_path.clone()) {
                        let mut new_dir_key = dir_key.clone();
                        new_dir_key.push(file_name_os_str);
                        subdirs.push((full_path, new_dir_key, depth + 1));
                    }
                }
                Some(_) | None => {
                    file_names.push(full_path);
                }
            }
        }

        let dir_size = if !args.block_size.is_empty() {
            calculate_directory_sizes(
                &dir_key,
                &file_names,
                args.a,
                args.summarize,
                threshold_size,
                &current_dir,
                &mut writer,
                &mut seen_inodes,
                count_links,
                &args.block_size,
            )?
        } else {
            calculate_directory_size_default(
                &dir_key,
                &file_names,
                args.human_readable,
                args.bytes,
                args.summarize,
                args.a,
                threshold_size,
                count_links,
                &mut writer,
                &mut seen_inodes,
                &current_dir,
            )
        };

        total_size += dir_size;
        dir_stack.extend(subdirs.into_iter().rev());
    }
    Ok(total_size)
}

fn main() -> Result<()> {
    let g_args = handle_args();
    let base_dir = g_args.x.as_ref().unwrap_or(&g_args.path);
    let depth = g_args.depth.unwrap_or(0);
    let current_dir = env::current_dir()?;

    let dir = if &current_dir == base_dir {
        format!(".")
    } else {
        format!("{}", base_dir.display())
    };

    let total_size = scan_directory_iter(
        base_dir,
        depth,
        g_args.x.as_deref(),
        g_args.xclude.as_deref(),
        g_args.l,
        g_args.clone(),
    )?;

    let formatted_size = if g_args.human_readable {
        get_file_sizes(None, Some(total_size))
    } else if !g_args.block_size.is_empty() {
        format_size(total_size, &g_args.block_size)?
    } else {
        total_size.to_string()
    };

    if g_args.summarize || depth == 0 {
        println!("{:<10} {}", formatted_size, dir);
    }

    if g_args.c && !g_args.summarize {
        println!("{:<10} total", formatted_size);
    }

    Ok(())
}
