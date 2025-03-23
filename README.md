# du-rs: A Rust Implementation of the `du` Command

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

`du-rs` is a Rust reimplementation of the classic Unix `du` (disk usage) command. This project is both an exploration of Rust's systems programming capabilities and a functional alternative to the traditional `du` utility.

> **Note:** This project is under active development. Performance isn’t great yet, and the code could `definitely` be better.

## Overview

The `du` command estimates file space usage. This Rust implementation maintains compatibility with many of the original `du` command's options while leveraging Rust's memory safety and performance benefits.

## Features

- Display disk usage of files and directories
- Human-readable output formats (-h)
- Multiple block size options (-B)
- Threshold filtering to show only items above a certain size (-t)
- File system traversal limitations (-x)
- Path exclusion capability (-X)
- Depth-limited directory scanning (-d)
- Summary mode for compact output (-s)
- Optional display of hidden files (-a)
- Bytes display mode (-b)

## Usage

```
Usage: du-rs [OPTIONS] [PATH]
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
  -X, --exclude-from PATH    Exclude paths from a file
```

## Examples

```bash
# Display disk usage in human-readable format
du-rs -h /path/to/directory

# Show only the total summary
du-rs -s /path/to/directory

# Display usage with a custom block size
du-rs -BM /path/to/directory

# Show only files larger than 1MB
du-rs -t 1M /path/to/directory

# Scan only up to a depth of 2 directories
du-rs -d 2 /path/to/directory
```

## Implementation Details

This implementation uses Rust's standard library and the `nix` crate to interact with Unix-like systems. Key features include:

- Safe handling of file and directory operations
- Efficient directory traversal using iterative scanning rather than recursion
- Proper handling of file system boundaries
- Support for unit conversions (K, M, G, T, etc.)
- Customizable block size settings

## Building from Source

```bash
# Clone the repository
git clone https://github.com/vamsi200/du-rs.git
cd du-rs

# Build the project
cargo build --release

# Run the executable
./target/release/du-rs
```

## Dependencies
- [nix](https://crates.io/crates/nix): Rust friendly bindings to *nix APIs
