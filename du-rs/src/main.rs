use crate::stat::lstat;
use dashmap::DashMap;
use fxhash::{FxHashMap, FxHashSet};
use indexmap::IndexMap;
use nix::dir::Dir;
use nix::{
    fcntl::OFlag,
    sys::stat::{self, Mode},
};
use rayon::prelude::*;
use std::ffi::OsStr;
use std::{
    collections::{HashMap, HashSet},
    env,
    fmt::Write,
    os::fd::RawFd,
    path::{Path, PathBuf},
    process::exit,
    sync::{
        atomic::{AtomicI64, Ordering},
        mpsc, Arc, Mutex,
    },
};

type Cresult<T> = std::result::Result<T, Box<dyn std::error::Error>>;
use anyhow::{Context, Result};
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
    }

    Err("-B requires a valid argument".into())
}

fn calculate_directory_sizes<'a>(
    dir: &'a IndexMap<PathBuf, Vec<PathBuf>>,
    show_all: bool,
    threshold_value: i64,
    c_dir: &'a Path,
    output_sender: &'a Arc<Mutex<std::sync::mpsc::Sender<String>>>,
    total_size: &'a AtomicI64,
) -> DashMap<&'a Path, i64> {
    let dir_sizes: DashMap<&Path, i64> = DashMap::new();
    let get_size: fn(&FileStats) -> i64 = FileStats::disk_usage_bytes;
    let batch = Vec::new();

    dir.par_iter().for_each(|(dir_path, files)| {
        let dir_size = get_size(&FileStats::from(dir_path));
        total_size.fetch_add(dir_size, Ordering::Relaxed);

        let file_size_sum = files
            .par_iter()
            .map(|file| {
                let file_size = get_size(&FileStats::from(file));
                total_size.fetch_add(file_size, Ordering::Relaxed);

                if show_all && file_size >= threshold_value {
                    if let Ok(formatted_file_size) = format_size(file_size, "human") {
                        let relative_path = file.strip_prefix(c_dir).unwrap_or(file);
                        batch.clone().push(format!(
                            "{:<10} ./{}",
                            formatted_file_size,
                            relative_path.display()
                        ));

                        if batch.len() > 10 {
                            let _ = output_sender.lock().unwrap().send(batch.join("\n"));
                            batch.clone().clear();
                        }
                    }
                }
                file_size
            })
            .sum::<i64>();

        dir_sizes.insert(dir_path.as_path(), dir_size + file_size_sum);
    });

    dir_sizes
}

fn send_directory_sizes(
    dir_sizes: DashMap<&Path, i64>,
    c_dir: &Path,
    threshold_value: i64,
    output_sender: &Arc<Mutex<std::sync::mpsc::Sender<String>>>,
    arg: &str,
    total_size: &AtomicI64,
) -> Cresult<()> {
    let mut sorted_dirs: Vec<_> = dir_sizes.iter().map(|entry| *entry.key()).collect();

    if sorted_dirs.len() < 10_000 {
        sorted_dirs.sort_unstable_by_key(|a| std::cmp::Reverse(a.as_os_str()));
    } else {
        sorted_dirs.par_sort_unstable_by_key(|a| std::cmp::Reverse(a.as_os_str()));
    }

    let mut dir_sizes_map: FxHashMap<&Path, i64> = dir_sizes.into_iter().collect();

    for dir_path in &sorted_dirs {
        if let Some(parent) = dir_path.parent() {
            if let Some(&dir_size) = dir_sizes_map.get(dir_path) {
                *dir_sizes_map.entry(parent).or_insert(0) += dir_size;
            }
        }
    }

    let mut output_buffer = String::new();

    for dir_path in sorted_dirs {
        if let Some(&dir_size) = dir_sizes_map.get(dir_path) {
            let dir_relative_path = dir_path.strip_prefix(c_dir).unwrap_or(dir_path);
            if dir_relative_path != Path::new(".") && dir_size >= threshold_value {
                output_buffer.clear();
                let formatted_dir_size = format_size(dir_size, arg)?;
                write!(
                    output_buffer,
                    "{:<10} {}",
                    formatted_dir_size,
                    dir_relative_path.display()
                )
                .map_err(|e| format!("Failed to write to buffer: {}", e))?;
                let _ = output_sender.lock().unwrap().send(output_buffer.clone());
            }
        }
    }

    let formatted_total = format_size(total_size.load(Ordering::Relaxed), arg)?;
    output_sender
        .lock()
        .unwrap()
        .send(format!("{:<10} ./", formatted_total))
        .unwrap();

    Ok(())
}

fn format_file_size(
    dir: &IndexMap<PathBuf, Vec<PathBuf>>,
    arg: &str,
    show_all: bool,
    threshold: String,
    output_sender: Arc<Mutex<std::sync::mpsc::Sender<String>>>,
) -> Cresult<String> {
    let threshold_value = parse_size_to_bytes(&threshold).unwrap_or(0);
    let c_dir =
        env::current_dir().map_err(|e| format!("Failed to get current directory: {}", e))?;
    let total_size = AtomicI64::new(0);

    let dir_sizes = calculate_directory_sizes(
        dir,
        show_all,
        threshold_value,
        &c_dir,
        &output_sender,
        &total_size,
    );
    send_directory_sizes(
        dir_sizes,
        &c_dir,
        threshold_value,
        &output_sender,
        arg,
        &total_size,
    )?;

    format_size(total_size.load(Ordering::Relaxed), arg)
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

fn calculate_directory_size_default(
    dir: &IndexMap<PathBuf, Vec<PathBuf>>,
    format: bool,
    is_bytes: bool,
    r_files: bool,
    threshold: String,
    l_arg: bool,
    output_sender: Arc<Mutex<std::sync::mpsc::Sender<String>>>,
) -> i64 {
    let size_format = if is_bytes {
        SizeFormat::Bytes
    } else if format {
        SizeFormat::HumanReadable
    } else {
        SizeFormat::Blocks
    };

    let c_dir = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let threshold_value = parse_size_to_bytes(&threshold).unwrap_or(0);
    let threshold = if !is_bytes && !format {
        threshold_value / 1024
    } else {
        threshold_value
    };

    let total_size = AtomicI64::new(0);
    let counted_inodes = Arc::new(Mutex::new(FxHashSet::default()));
    let dir_sizes = DashMap::new();

    dir.par_iter().for_each(|(dir_path, file_names)| {
        let dir_stats = FileStats::from(dir_path);
        let initial_dir_size = size_format.get_dir_size(&dir_stats);
        total_size.fetch_add(initial_dir_size, Ordering::Relaxed);

        let file_sizes_sum = file_names
            .par_iter()
            .map(|file| {
                let file_stats = FileStats::from(file);
                let file_size = size_format.get_file_size(&file_stats);

                if l_arg {
                    if let Ok(metadata) = lstat(file) {
                        let inode = metadata.st_ino;
                        let mut counted = counted_inodes.lock().unwrap();
                        if counted.insert(inode) {
                            total_size.fetch_add(file_size, Ordering::Relaxed);
                        }
                    }
                }

                if r_files && file_size >= threshold {
                    let relative_path = file.strip_prefix(&c_dir).unwrap_or(file);
                    let display_path = if file.starts_with(&c_dir) {
                        format!("./{}", relative_path.to_string_lossy())
                    } else {
                        relative_path.to_string_lossy().into_owned()
                    };

                    let formatted_size = if format {
                        get_file_sizes(None, Some(file_size))
                    } else {
                        file_size.to_string()
                    };

                    let line = format!("{:<10} {}", formatted_size, display_path);
                    output_sender.lock().unwrap().send(line).unwrap();
                }

                file_size
            })
            .sum::<i64>();

        dir_sizes.insert(dir_path.as_path(), initial_dir_size + file_sizes_sum);
    });

    // converting DashMap to FxHashMap for sequential processing
    let dir_sizes: FxHashMap<_, _> = dir_sizes
        .into_read_only()
        .iter()
        .map(|(k, v)| (*k, *v))
        .collect();

    // Update parent directories
    let mut sorted_dirs: Vec<_> = dir_sizes.keys().cloned().collect();
    sorted_dirs.par_sort_unstable_by_key(|a| std::cmp::Reverse(a.as_os_str()));

    let mut dir_sizes = dir_sizes;
    for dir_path in &sorted_dirs {
        if let Some(parent) = dir_path.parent() {
            if let Some(&dir_size) = dir_sizes.get(dir_path) {
                *dir_sizes.entry(parent).or_insert(0) += dir_size;
            }
        }
    }

    for dir_path in &sorted_dirs {
        let dir_size = *dir_sizes.get(dir_path).unwrap_or(&0);
        if dir_size < threshold {
            continue;
        }

        let relative_path = match dir_path.strip_prefix(&c_dir) {
            Ok(rel) => rel.to_string_lossy().into_owned(),
            Err(_) => dir_path.to_str().unwrap_or("").to_string(),
        };

        let formatted_size = if format {
            get_file_sizes(None, Some(dir_size))
        } else {
            dir_size.to_string()
        };

        let line = format!("{:<10} {}", formatted_size, relative_path);
        output_sender.lock().unwrap().send(line).unwrap();
    }

    total_size.into_inner()
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
) -> Result<IndexMap<PathBuf, Vec<PathBuf>>> {
    use std::os::unix::ffi::OsStrExt;
    let current_dir = env::current_dir().context("Failed to get current directory")?;
    let cd = current_dir == root_dir;
    let mut dir_stack = Vec::new();
    let mut dir_map = IndexMap::new();
    let mut visited = FxHashSet::default();

    let root_dev = if x_option.is_some() {
        Some(
            nix::sys::stat::stat(root_dir)
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
    let use_exclusion = is_exclude.is_some();

    while let Some((absolute_path, dir_key, depth)) = dir_stack.pop() {
        if let Some(root_dev) = root_dev {
            let sub_dir_dev = nix::sys::stat::stat(&absolute_path)
                .with_context(|| format!("Failed to stat {:?}", absolute_path))?
                .st_dev;
            if root_dev != sub_dir_dev {
                continue;
            }
        }

        let mut file_names = Vec::new();
        let mut subdirs = Vec::new();

        let open_dir = nix::fcntl::open(&absolute_path, OFlag::O_RDONLY, Mode::empty())
            .ok()
            .and_then(|fd| Dir::from_fd(fd).ok());

        if let Some(open_dir) = open_dir {
            for entry in open_dir {
                let entry = entry
                    .with_context(|| format!("Error reading directory {:?}", absolute_path))?;
                let file_name_os_str = OsStr::from_bytes(entry.file_name().to_bytes());

                if file_name_os_str == "." || file_name_os_str == ".." {
                    continue;
                }

                let full_path = absolute_path.join(file_name_os_str);
                if use_exclusion
                    && (exclusion_paths.contains(&full_path)
                        || full_path
                            .extension()
                            .map_or(false, |ext| exclusion_patterns.contains(ext)))
                {
                    continue;
                }

                match entry.file_type() {
                    Some(nix::dir::Type::Directory) => {
                        if !no_depth && depth >= max_depth {
                            continue;
                        }
                        if visited.insert(full_path.to_owned()) {
                            let mut new_dir_key = dir_key.clone();
                            new_dir_key.push(file_name_os_str);
                            subdirs.push((full_path, new_dir_key, depth + 1));
                        }
                    }
                    Some(nix::dir::Type::File) => {
                        file_names.push(full_path);
                    }
                    _ => {}
                }
            }
        }

        dir_stack.extend(subdirs.into_iter().rev());
        dir_map.insert(dir_key, file_names);
    }

    Ok(dir_map)
}
#[cfg(test)]
mod tests;
fn main() -> Result<()> {
    use std::io::{self, BufWriter, Write};

    let g_args = handle_args();

    let (tx, rx) = mpsc::channel();
    let shared_output = Arc::new(Mutex::new(tx));

    let base_dir = g_args.x.as_ref().unwrap_or(&g_args.path);
    let depth = g_args.depth.unwrap_or(0);

    let dir_map = scan_directory_iter(
        base_dir,
        depth,
        g_args.x.as_deref(),
        g_args.xclude.as_deref(),
    )?;

    let output_thread = std::thread::spawn(move || {
        let mut output = BufWriter::new(io::stdout().lock());
        for line in rx {
            if writeln!(output, "{}", line).is_err() {
                break;
            }
        }
        let _ = output.flush();
    });

    if !g_args.block_size.is_empty() {
        format_file_size(
            &dir_map,
            &g_args.block_size,
            g_args.a,
            g_args.threshold.unwrap_or_default(),
            shared_output.clone(),
        )
        .unwrap();

        if g_args.summarize || g_args.total {
            shared_output
                .lock()
                .unwrap()
                .send(format!("{:<10}  .", " "))
                .unwrap();
        }
    } else {
        let total_dir_size = calculate_directory_size_default(
            &dir_map,
            g_args.human_readable,
            g_args.bytes,
            g_args.a,
            g_args.threshold.unwrap_or_default(),
            g_args.l,
            shared_output.clone(),
        );

        if g_args.summarize || g_args.total {
            let output = get_file_sizes(None, Some(total_dir_size));
            let label = if g_args.summarize { "." } else { "total" };
            let size_display = if g_args.human_readable {
                output
            } else {
                total_dir_size.to_string()
            };
            shared_output
                .lock()
                .unwrap()
                .send(format!("{:<10} {}", size_display, label))
                .unwrap();
        }
    }

    drop(shared_output);
    output_thread.join().unwrap();

    Ok(())
}
