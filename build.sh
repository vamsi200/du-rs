#!/bin/bash

set -e
git clone https://github.com/vamsi200/du-rs.git
cd du-rs/
cargo build --release
