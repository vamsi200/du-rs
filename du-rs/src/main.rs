#![allow(unused_imports)]
#![allow(dead_code)]
#![allow(unused_variables)]
use nix::fcntl::OFlag;
use nix::{sys::stat::Mode, *};
use std::clone;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::{env, error::Error, result};

type MyResult<T> = result::Result<T, Box<dyn Error>>;

//fn scan_directory(dir: &Path) -> HashMap<PathBuf, String> {
//    //Returns Full path, i.e.. PathBuf and a string saying that its a file or directory
//    let open_dir = nix::dir::Dir::open(dir, OFlag::O_RDONLY, Mode::empty()).unwrap();
//    let mut output = HashMap::new();
//    for res in open_dir {
//        match res {
//            Ok(entry) => {
//                let file_name = entry.file_name().to_string_lossy();
//                if file_name == "." && file_name.len() == 1
//                    || file_name == ".." && file_name.len() == 2
//                {
//                    continue;
//                }
//                let full_path = dir.join(entry.file_name().to_string_lossy().as_ref());
//
//                let file_type = match entry.file_type() {
//                    Some(nix::dir::Type::Directory) => "dir".to_string(),
//                    Some(nix::dir::Type::File) => "file".to_string(),
//                    _ => "unknown".to_string(),
//                };
//                output.insert(full_path, file_type);
//            }
//            Err(e) => eprintln!("Error {}", e),
//        }
//    }
//    output
//}

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

fn format_file_size(file_path: &Path, arg: String) -> MyResult<String> {
    //Returns file size based on argument given
    if let Ok(res) = nix::sys::stat::stat(file_path) {
        let bytes = res.st_blocks * 512;
        let output = match arg.as_str() {
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
    Err("Failed to get file size".into())
}

fn count_files(dir: &Path) -> u64 {
    todo!()
}

fn calculate_total_dir_size(dir: BTreeMap<PathBuf, Vec<PathBuf>>) {
    let mut total: i64 = 0;
    let mut summarize_size: i64 = 0;
    let current_dir = env::current_dir().unwrap();
    for (dir_path, files) in dir.clone() {
        match dir.get(&dir_path) {
            Some(d) => {
                for file in d {
                    let fb = get_file_size(Some(file), None);
                    let bytes = fb.bytes;
                    let f_bytes = fb.formatted;
                    let stripped_file = file
                        .strip_prefix(current_dir.clone())
                        .unwrap_or(file)
                        .to_path_buf();
                    println!("{:<10}./{}", f_bytes, stripped_file.display());
                    total += bytes;
                }
            }
            None => println!("wut?"),
        }
        let total_dir_formatted = get_file_size(None, Some(total));
        let tdf = total_dir_formatted.formatted;
        summarize_size += total_dir_formatted.bytes;

        println!("{:<10}{}", tdf, dir_path.display());
    }
    let something = get_file_size(None, Some(summarize_size));
    let s = something.formatted;
    println!("Entirety : {}", s);
}

//fn scan_directory_recursive(dir: &Path) {
//    let output = scan_directory(&dir);
//    for (path, file_type) in output {
//        if file_type == "dir" {
//            println!("\nDirectory: {:?}", path);
//            scan_directory_recursive(&path);
//        }
//    }
//}

fn scan_directory_iter(root_dir: &Path, max_depth: i32) -> BTreeMap<PathBuf, Vec<PathBuf>> {
    let mut dir_stack = VecDeque::new();
    let mut dir_map = BTreeMap::new();
    let no_depth = max_depth == 0;
    dir_stack.push_back((root_dir.to_path_buf(), 0));
    while let Some((d_path, depth)) = dir_stack.pop_front() {
        let open_dir = match nix::dir::Dir::open(&d_path, OFlag::O_RDONLY, Mode::empty()) {
            Ok(dir) => dir,
            Err(e) => {
                //log to a file?
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
                    let full_path = d_path.join(file_name.as_ref());

                    match entry.file_type() {
                        Some(nix::dir::Type::Directory) => {
                            //when zero given f the checks..and proceed to push everything, ie.
                            //no_depth becomes true and so the other condition.
                            //if some value given then no_depth becomes false and now it depends on
                            //other check.. so it will push it until depth < max_depth.
                            if no_depth || depth < max_depth {
                                dir_stack.push_back((full_path.clone(), depth + 1));
                            }
                            dir_map.entry(d_path.clone()).or_insert_with(Vec::new);
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
        dir_map.insert(d_path, files);
    }
    dir_map
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

    //let mut output = scan_directory(&current_dir);
    //
    //for (path, file_type) in output {
    //    if file_type == "file" {
    //        let file_size = get_file_size_in_bytes(&path);
    //        println!(
    //            "filename: {:?} filesize: {file_size} bytes",
    //            path.file_name()
    //        );
    //        let human_readable_size = get_file_size(&path);
    //        println!("{human_readable_size}");
    //        let output = format_file_size(&path, "BQ".to_string());
    //        println!("{:?}", output);
    //    }
    //}
    //
    //scan_directory_recursive(&current_dir);
    //    let output = scan_directory_iter(&current_dir);
    //println!("{:?}", output);
    //for (path, file_type) in output {
    //    let file_size = get_file_size_in_bytes(path.clone());
    //    let dir_path;
    //    if file_type == "dir" {
    //        dir_path = path.clone();
    //        let c = current_dir.to_string_lossy().into_owned();
    //        let d = dir_path.strip_prefix(c);
    //        //println!(".{:?}", d);
    //    }
    //}
    let dir_map = scan_directory_iter(&current_dir, 0);
    calculate_total_dir_size(dir_map.clone());
    //let base_path = current_dir;
    //for (dir, files) in &dir_map {
    //    for file in files {
    //        let file_path = file.as_path();
    //        let file_size = get_file_size_in_bytes(file_path.to_owned());
    //        let relative_path = file_path
    //            .strip_prefix(base_path.clone())
    //            .unwrap_or(file_path);
    //        println!("{:<10}./{}", file_size, relative_path.display());
    //    }
    //
    //    let dir_size = get_file_size_in_bytes(dir.to_owned());
    //    let relative_dir = dir.strip_prefix(base_path.clone()).unwrap_or(dir);
    //    println!("{:<10}./{}", dir_size, relative_dir.display());
    //}
}
