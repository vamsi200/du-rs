use fxhash::FxHashSet;
use nix::dir::Dir;
use nix::fcntl::openat;
use nix::fcntl::{open, AtFlags};
use nix::sys::stat::{self, fstatat};
use nix::{fcntl::OFlag, sys::stat::Mode};
use std::ffi::OsStr;
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
use anyhow::{Context, Error, Result};
struct FileStats {
    size: i64,
    blocks: i64,
}

impl FileStats {
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

    let mut output = String::with_capacity(32);

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
#[repr(u8)]
enum SizeFormat {
    Bytes,
    HumanReadable,
    Blocks,
}

impl SizeFormat {
    #[inline]
    fn get_dir_size(&self, stats: &FileStats) -> i64 {
        match self {
            SizeFormat::Bytes => 0,
            SizeFormat::HumanReadable => stats.disk_usage_bytes(),
            SizeFormat::Blocks => stats.disk_usage_blocks(),
        }
    }

    #[inline]
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

fn process_directories(args: Args) -> Result<i64> {
    use fxhash::FxHashSet;
    use nix::sys::stat::stat;
    use std::ffi::OsStr;
    use std::io::{stdout, BufWriter, Write};

    let root_dir: &PathBuf = &args.path;
    let max_depth = args.depth.unwrap_or(0);
    let x_option = args.x;
    let xclude = args.xclude.as_deref();
    let count_hardlinks = args.count_hardlinks;
    let mut writer = BufWriter::new(stdout());
    let mut seen_inodes = FxHashSet::default();
    let follow_symlink = args.follow_symlinks;
    let threshold = args.threshold.as_deref().unwrap_or("0");

    let root_dev = if x_option.is_some() {
        Some(
            stat(root_dir)
                .context("Failed to get device ID of root directory")?
                .st_dev,
        )
    } else {
        None
    };

    let mut exclusion_paths = FxHashSet::default();
    let mut exclusion_patterns = FxHashSet::default();
    if let Some(exclude_path) = xclude {
        for s in exclude_list(exclude_path) {
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

    let fd = if !count_hardlinks {
        match open(
            root_dir,
            OFlag::O_DIRECTORY | OFlag::O_RDONLY | OFlag::O_NOFOLLOW,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(_) => return Ok(0),
        }
    } else {
        match open(
            root_dir,
            OFlag::O_DIRECTORY | OFlag::O_RDONLY,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(_) => return Ok(0),
        }
    };

    let threshold_bytes = parse_size_to_bytes(threshold).unwrap_or(0);
    let threshold = if !args.bytes && !args.human_readable {
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
    let current_dir = env::current_dir()?;
    let is_current_dir = root_dir == &current_dir || root_dir.as_os_str() == OsStr::new(".");

    let mut path_bytes = Vec::with_capacity(4096);

    if is_current_dir {
        path_bytes.extend_from_slice(b".");
    } else {
        let root_bytes = root_dir.as_os_str().as_bytes();
        path_bytes.extend_from_slice(root_bytes);
    }

    let total = if !args.block_size.is_empty() {
        recursive_dir_iter(
            fd,
            0,
            max_depth,
            root_dev,
            &exclusion_paths,
            &exclusion_patterns,
            args.human_readable,
            args.bytes,
            args.summarize,
            args.a,
            threshold,
            follow_symlink,
            count_hardlinks,
            &mut writer,
            &mut seen_inodes,
            Some(&args.block_size),
            xclude,
            &size_format,
            &mut path_bytes,
        )?
    } else {
        recursive_dir_iter(
            fd,
            0,
            max_depth,
            root_dev,
            &exclusion_paths,
            &exclusion_patterns,
            args.human_readable,
            args.bytes,
            args.summarize,
            args.a,
            threshold,
            follow_symlink,
            count_hardlinks,
            &mut writer,
            &mut seen_inodes,
            None,
            xclude,
            &size_format,
            &mut path_bytes,
        )?
    };

    writer.flush()?;
    Ok(total)
}

#[inline(always)]
fn recursive_dir_iter(
    raw_fd: RawFd,
    current_depth: i32,
    max_depth: i32,
    root_dev: Option<u64>,
    exclusion_paths: &FxHashSet<PathBuf>,
    exclusion_patterns: &FxHashSet<std::ffi::OsString>,
    format: bool,
    is_bytes: bool,
    summarize: bool,
    list_files: bool,
    threshold_size: i64,
    follow_sysmlink: bool,
    count_hard_link: bool,
    writer: &mut BufWriter<std::io::Stdout>,
    seen_inodes: &mut FxHashSet<(u64, u64)>,
    block_size: Option<&str>,
    exclude_path: Option<&Path>,
    size_format: &SizeFormat,
    path_bytes: &mut Vec<u8>,
) -> Result<i64> {
    use std::os::unix::ffi::OsStrExt;

    let mut total_size: i64 = 0;
    let use_exclusion = exclude_path.is_some();

    let meta = if !follow_sysmlink {
        if let Ok(meta) = fstatat(Some(raw_fd), OsStr::new("."), AtFlags::AT_SYMLINK_NOFOLLOW) {
            meta
        } else {
            return Ok(0);
        }
    } else {
        if let Ok(meta) = fstatat(Some(raw_fd), OsStr::new("."), AtFlags::empty()) {
            meta
        } else {
            return Ok(0);
        }
    };

    if let Some(dev) = root_dev {
        if meta.st_dev != dev {
            return Ok(0);
        }
    }

    let file_stats = FileStats {
        size: meta.st_size,
        blocks: meta.st_blocks,
    };
    total_size += size_format.get_dir_size(&file_stats);

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
        let child_meta = if !follow_sysmlink {
            match fstatat(Some(raw_fd), file_name_osstr, AtFlags::AT_SYMLINK_NOFOLLOW) {
                Ok(m) => m,
                Err(_) => continue,
            }
        } else {
            match fstatat(Some(raw_fd), file_name_osstr, AtFlags::empty()) {
                Ok(m) => m,
                Err(_) => continue,
            }
        };

        if use_exclusion
            && (exclusion_paths.contains(&PathBuf::from(file_name_osstr.to_owned()))
                || PathBuf::from(file_name_osstr.to_owned())
                    .extension()
                    .map_or(false, |ext| exclusion_patterns.contains(ext)))
        {
            continue;
        }

        if !count_hard_link && child_meta.st_nlink > 1 {
            let inode = (child_meta.st_dev as u64, child_meta.st_ino as u64);
            if !seen_inodes.insert(inode) {
                continue;
            }
        }

        match entry.file_type() {
            Some(nix::dir::Type::Directory) => {
                if max_depth > 0 && current_depth >= max_depth {
                    continue;
                }

                let sub_fd = if !follow_sysmlink {
                    match openat(
                        Some(raw_fd),
                        file_name_osstr,
                        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW,
                        Mode::empty(),
                    ) {
                        Ok(fd) => fd,
                        Err(_) => continue,
                    }
                } else {
                    match openat(
                        Some(raw_fd),
                        file_name_osstr,
                        OFlag::O_RDONLY | OFlag::O_DIRECTORY,
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
                    max_depth,
                    root_dev,
                    exclusion_paths,
                    exclusion_patterns,
                    format,
                    is_bytes,
                    summarize,
                    list_files,
                    threshold_size,
                    follow_sysmlink,
                    count_hard_link,
                    writer,
                    seen_inodes,
                    block_size,
                    exclude_path,
                    size_format,
                    path_bytes,
                )?;
                if !summarize && subdir_size >= threshold_size {
                    let path_osstr = OsStr::from_bytes(&path_bytes);
                    let path_display = path_osstr.to_string_lossy();

                    if let Some(bs) = block_size {
                        let formatted = format_size(subdir_size, bs)?;
                        writeln!(writer, "{:<10} {}", formatted, path_display)?;
                    } else if format {
                        let formatted = get_file_sizes(None, Some(subdir_size));
                        writeln!(writer, "{:<10} {}", formatted, path_display)?;
                    } else {
                        writeln!(writer, "{:<10} {}", subdir_size, path_display)?;
                    }
                }

                total_size += subdir_size;

                path_bytes.truncate(saved_len);
            }

            _ => {
                let file_stats = FileStats {
                    size: child_meta.st_size,
                    blocks: child_meta.st_blocks,
                };

                let file_size = size_format.get_file_size(&file_stats);
                total_size += file_size;

                if list_files && !summarize && file_size >= threshold_size {
                    let saved_len = path_bytes.len();

                    if !path_bytes.is_empty() {
                        path_bytes.push(b'/');
                    }
                    path_bytes.extend_from_slice(file_name_bytes);

                    let path_osstr = OsStr::from_bytes(&path_bytes);
                    let path_display = path_osstr.to_string_lossy();

                    if let Some(bs) = block_size {
                        let formatted = format_size(file_size, bs)?;
                        writeln!(writer, "{:<10} {}", formatted, path_display)?;
                    } else if format {
                        let formatted = get_file_sizes(None, Some(file_size));
                        writeln!(writer, "{:<10} {}", formatted, path_display)?;
                    } else {
                        writeln!(writer, "{:<10} {}", file_size, path_display)?;
                    }

                    path_bytes.truncate(saved_len);
                }
            }
        }
    }

    Ok(total_size)
}

fn main() -> Result<()> {
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
