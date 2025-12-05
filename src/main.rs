use fxhash::FxHashSet;
use nix::dir::Dir;
use nix::fcntl::openat;
use nix::fcntl::AtFlags;
use nix::sys::stat::{self, fstatat};
use nix::{fcntl::OFlag, sys::stat::Mode};
use std::ffi::{OsStr, OsString};
use std::io::{BufWriter, Write};
use std::os::unix::ffi::OsStrExt;
use std::{
    collections::{HashMap, HashSet},
    env,
    os::fd::RawFd,
    path::{Path, PathBuf},
    process::exit,
};

type Cresult<T> = anyhow::Result<T, anyhow::Error>;
use anyhow::{Context, Error};
struct FileStats {
    size: i64,
    blocks: i64,
}

impl FileStats {
    #[inline]
    fn size_in_bytes(&self) -> i64 {
        self.size
    }
    #[inline]
    fn disk_usage_blocks(&self) -> i64 {
        (self.blocks * 512) / 1024
    }
    #[inline]
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

    let mut output = String::with_capacity(32);

    if bytes < 1024 {
        return format!("{bytes}B");
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
    let _ = write!(output, "{:.1}{}", value, unit);
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
#[repr(u8)]
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
    count_hardlinks: bool,
    follow_symlinks: bool,
    c: bool,
}

fn handle_args() -> Args {
    let mut arguments = env::args().skip(1);
    let mut path = match env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("du-rs: cannot determine current directory: {e}");
            std::process::exit(1);
        }
    };

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
    let mut follow_symlinks = false;
    let mut c = false;
    let mut count_hardlinks = false;
    while let Some(arg) = arguments.next() {
        match arg.as_str() {
            "--help" => print_help(),
            "-h" | "--human-readable" => human_readable = true,
            "-a" | "--all" => a = true,
            "-L" => follow_symlinks = true,
            "-l" => count_hardlinks = true,
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
        count_hardlinks,
        follow_symlinks,
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum FileContent {
    Path(PathBuf),
    Pattern(String),
}

fn exclude_list(file: &Path) -> HashSet<FileContent> {
    let file_fd = match nix::fcntl::open(file, OFlag::O_RDONLY, Mode::empty()) {
        Ok(fd) => fd,
        Err(e) => {
            eprintln!("du-rs: cannot access '{}': {}", file.display(), e);
            return HashSet::new();
        }
    };

    let mut buffer = [0u8; 1024];
    let mut content = String::new();
    let mut hs = HashSet::new();

    loop {
        let bytes_read = match nix::unistd::read(file_fd, &mut buffer) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("du-rs: failed reading '{}': {}", file.display(), e);
                return HashSet::new();
            }
        };

        if bytes_read == 0 {
            break;
        }
        content.push_str(&String::from_utf8_lossy(&buffer[..bytes_read]));
    }
    if let Err(e) = nix::unistd::close(file_fd) {
        eprintln!("du-rs: failed to close file {}: {}", file_fd, e);
    }

    let current_dir = match env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("du-rs: cannot determine current directory: {e}");
            std::process::exit(1);
        }
    };

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

struct TraversalConfig {
    max_depth: i32,
    root_dev: Option<u64>,
    exclusion_paths: Option<FxHashSet<PathBuf>>,
    exclusion_patterns: Option<FxHashSet<OsString>>,
    format: bool,
    summarize: bool,
    list_files: bool,
    threshold_size: i64,
    count_hard_link: bool,
    block_size: Option<String>,
    size_format: SizeFormat,
    open_flag: OFlag,
    at_flag: AtFlags,
}

fn process_directories(args: Args) -> Cresult<i64> {
    use fxhash::FxHashSet;
    use nix::fcntl::open;
    use nix::sys::stat::{stat, Mode};
    use std::env;
    use std::ffi::{OsStr, OsString};
    use std::io::{stdout, BufWriter, Write};

    let root_dir: &PathBuf = &args.path;
    let max_depth = args.depth.unwrap_or(0);

    let follow_symlink = args.follow_symlinks;

    let open_flag = if !follow_symlink {
        OFlag::O_DIRECTORY | OFlag::O_RDONLY | OFlag::O_NOFOLLOW
    } else {
        OFlag::O_DIRECTORY | OFlag::O_RDONLY
    };

    let at_flag = if !follow_symlink {
        AtFlags::AT_SYMLINK_NOFOLLOW
    } else {
        AtFlags::empty()
    };

    let fd = match open(root_dir, open_flag, Mode::empty()) {
        Ok(fd) => fd,
        Err(_) => return Ok(0),
    };

    let root_dev = if args.x.is_some() {
        stat(root_dir)
            .context("Failed to get device ID of root directory")
            .ok()
            .map(|s| s.st_dev)
    } else {
        None
    };

    let (exclusion_paths, exclusion_patterns) = if let Some(exclude_path) = args.xclude.as_deref() {
        let mut paths = FxHashSet::default();
        let mut patterns = FxHashSet::default();

        for s in exclude_list(exclude_path) {
            match s {
                FileContent::Path(p) => {
                    paths.insert(p);
                }
                FileContent::Pattern(pt) => {
                    patterns.insert(OsString::from(pt));
                }
            }
        }
        (Some(paths), Some(patterns))
    } else {
        (None, None)
    };

    let threshold_bytes =
        parse_size_to_bytes(args.threshold.as_deref().unwrap_or("0")).unwrap_or(0);
    let threshold_size = if !args.bytes && !args.human_readable {
        threshold_bytes / 1024
    } else {
        threshold_bytes
    };

    let size_format = if !args.block_size.is_empty() {
        SizeFormat::HumanReadable
    } else if args.bytes {
        SizeFormat::Bytes
    } else if args.human_readable {
        SizeFormat::HumanReadable
    } else {
        SizeFormat::Blocks
    };

    let block_size = if args.block_size.is_empty() {
        None
    } else {
        Some(args.block_size.clone())
    };

    let config = TraversalConfig {
        max_depth,
        root_dev,
        exclusion_paths,
        exclusion_patterns,
        format: args.human_readable,
        summarize: args.summarize,
        list_files: args.a,
        threshold_size,
        count_hard_link: args.count_hardlinks,
        block_size,
        size_format,
        open_flag,
        at_flag,
    };

    let mut writer = BufWriter::new(stdout());
    let mut seen_inodes = FxHashSet::default();
    let mut path_bytes = Vec::with_capacity(4096);

    let current_dir = env::current_dir()?;
    let is_current_dir = root_dir == &current_dir || root_dir.as_os_str() == OsStr::new(".");

    if is_current_dir {
        path_bytes.extend_from_slice(b".");
    } else {
        use std::os::unix::ffi::OsStrExt;
        path_bytes.extend_from_slice(root_dir.as_os_str().as_bytes());
    }

    let total = recursive_dir_iter(
        fd,
        0,
        &config,
        &mut writer,
        &mut seen_inodes,
        &mut path_bytes,
    )?;

    writer.flush()?;

    Ok(total)
}

fn recursive_dir_iter(
    raw_fd: RawFd,
    current_depth: i32,
    config: &TraversalConfig,
    writer: &mut BufWriter<std::io::Stdout>,
    seen_inodes: &mut FxHashSet<(u64, u64)>,
    path_bytes: &mut Vec<u8>,
) -> Cresult<i64> {
    let mut total_size: i64 = 0;

    let meta = {
        if let Ok(meta) = fstatat(Some(raw_fd), OsStr::new("."), config.at_flag) {
            meta
        } else {
            return Ok(0);
        }
    };

    if let Some(dev) = config.root_dev {
        if meta.st_dev != dev {
            return Ok(0);
        }
    }

    let file_stats = FileStats {
        size: meta.st_size,
        blocks: meta.st_blocks,
    };
    total_size += config.size_format.get_dir_size(&file_stats);

    let dir = match Dir::from_fd(raw_fd) {
        Ok(d) => d,
        Err(_) => return Ok(total_size),
    };

    for entry in dir {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let file_name_bytes = entry.file_name().to_bytes();
        if file_name_bytes == b"." || file_name_bytes == b".." {
            continue;
        }

        let file_name_osstr = OsStr::from_bytes(file_name_bytes);
        let excluded = config.exclusion_paths.as_ref().map_or(false, |paths| {
            let file_path = Path::new(file_name_osstr);
            paths.contains(file_path)
        }) || config
            .exclusion_patterns
            .as_ref()
            .map_or(false, |patterns| {
                Path::new(file_name_osstr)
                    .extension()
                    .map_or(false, |ext| patterns.contains(ext))
            });

        if excluded {
            continue;
        }

        match entry.file_type() {
            Some(nix::dir::Type::Directory) => {
                if config.max_depth > 0 && current_depth >= config.max_depth {
                    continue;
                }

                let sub_fd = {
                    match openat(
                        Some(raw_fd),
                        file_name_osstr,
                        config.open_flag,
                        Mode::empty(),
                    ) {
                        Ok(fd) => fd,
                        Err(_) => continue,
                    }
                };

                let saved_len = path_bytes.len();

                if !path_bytes.is_empty() {
                    path_bytes.push(b'/');
                }
                path_bytes.extend_from_slice(file_name_bytes);

                let subdir_size = recursive_dir_iter(
                    sub_fd,
                    current_depth + 1,
                    config,
                    writer,
                    seen_inodes,
                    path_bytes,
                )?;
                if !config.summarize && subdir_size >= config.threshold_size {
                    write_to_stdout(
                        writer,
                        subdir_size,
                        &path_bytes,
                        config.block_size.as_deref(),
                        config.format,
                    )?;
                }

                total_size += subdir_size;

                path_bytes.truncate(saved_len);
            }

            _ => {
                let child_meta = {
                    match fstatat(Some(raw_fd), file_name_osstr, config.at_flag) {
                        Ok(m) => m,
                        Err(_) => continue,
                    }
                };

                if !config.count_hard_link && child_meta.st_nlink > 1 {
                    let inode = (child_meta.st_dev, child_meta.st_ino);
                    if !seen_inodes.insert(inode) {
                        continue;
                    }
                }

                let file_stats = FileStats {
                    size: child_meta.st_size,
                    blocks: child_meta.st_blocks,
                };

                let file_size = config.size_format.get_file_size(&file_stats);
                total_size += file_size;

                if config.list_files && !config.summarize && file_size >= config.threshold_size {
                    let saved_len = path_bytes.len();

                    if !path_bytes.is_empty() {
                        path_bytes.push(b'/');
                    }
                    path_bytes.extend_from_slice(file_name_bytes);

                    write_to_stdout(
                        writer,
                        file_size,
                        &path_bytes,
                        config.block_size.as_deref(),
                        config.format,
                    )?;

                    path_bytes.truncate(saved_len);
                }
            }
        }
    }

    Ok(total_size)
}

fn write_to_stdout(
    writer: &mut BufWriter<std::io::Stdout>,
    size: i64,
    path_bytes: &[u8],
    block_size: Option<&str>,
    format: bool,
) -> Cresult<()> {
    let size_str = if let Some(bs) = block_size {
        format_size(size, bs)?
    } else if format {
        get_file_sizes(None, Some(size))
    } else {
        let mut buffer = itoa::Buffer::new();
        buffer.format(size).to_owned()
    };

    let size_len = size_str.len();
    writer.write_all(size_str.as_bytes())?;

    if size_len < 10 {
        static SPACES: &[u8] = b"          ";
        writer.write_all(&SPACES[..10 - size_len])?;
    }

    writer.write_all(b" ")?;

    writer.write_all(path_bytes)?;

    writer.write_all(b"\n")?;

    Ok(())
}

fn main() -> Cresult<()> {
    let g_args = handle_args();
    let base_dir = g_args.x.as_ref().unwrap_or(&g_args.path);
    let current_dir = env::current_dir()?;

    let dir = if &current_dir == base_dir {
        format!(".")
    } else {
        format!("{}", base_dir.display())
    };

    let total_size = process_directories(g_args.clone())?;
    let formatted_size = if g_args.human_readable {
        get_file_sizes(None, Some(total_size))
    } else if !g_args.block_size.is_empty() {
        format_size(total_size, &g_args.block_size)?
    } else {
        total_size.to_string()
    };
    if g_args.summarize {
        println!("{:<10} {}", formatted_size, dir);
    } else if g_args.c && !g_args.summarize || g_args.total {
        println!("{:<10} total", formatted_size);
    } else {
        println!("{:<10} {}", formatted_size, dir);
    }

    Ok(())
}
