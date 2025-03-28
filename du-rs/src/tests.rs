#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::io::Write;
    use std::process::Command;

    fn setup_test_environment() {
        let _ = fs::create_dir_all("test_env/test_dir2");
        let _ = fs::create_dir_all("test_env/test1");
        let mut file = File::create("test_env/test_dir2/test.txt").unwrap();
        writeln!(file, "hello").unwrap();

        let mut file = File::create("test_env/test.txt").unwrap();
        writeln!(file, "world").unwrap();

        let mut file = File::create("test_env/test2.txt").unwrap();
        writeln!(file, "test").unwrap();
    }

    #[test]
    fn test_du_ah() {
        setup_test_environment();

        let output = Command::new("/home/vamsi/scripts/du-rs/du-rs/target/release/du-rs")
            .arg("-ah")
            .arg("test_env")
            .output()
            .expect("Failed to execute process");

        let stdout = String::from_utf8_lossy(&output.stdout);

        let expected_output = "\
4.0K       test_env/test2.txt
4.0K       test_env/test.txt
4.0K       test_env/test_dir2/test.txt
8.0K       test_env/test_dir2
4.0K       test_env/test1
24.0K      test_env
";

        assert_eq!(stdout.trim(), expected_output.trim());
    }

    #[test]
    fn test_du_no_args() {
        setup_test_environment();

        let output = Command::new("/home/vamsi/scripts/du-rs/du-rs/target/release/du-rs")
            .arg("test_env")
            .output()
            .expect("Failed to execute process");

        let stdout = String::from_utf8_lossy(&output.stdout);

        let expected_output = "\
8          test_env/test_dir2
4          test_env/test1
24         test_env
";

        assert_eq!(stdout.trim(), expected_output.trim());
    }

    #[test]
    fn test_du_a() {
        setup_test_environment();

        let output = Command::new("/home/vamsi/scripts/du-rs/du-rs/target/release/du-rs")
            .arg("-a")
            .arg("test_env")
            .output()
            .expect("Failed to execute process");

        let stdout = String::from_utf8_lossy(&output.stdout);

        let expected_output = "\
4          test_env/test2.txt
4          test_env/test.txt
4          test_env/test_dir2/test.txt
8          test_env/test_dir2
4          test_env/test1
24         test_env
";

        assert_eq!(stdout.trim(), expected_output.trim());
    }
    #[test]
    fn test_du_b() {
        setup_test_environment();

        let output = Command::new("/home/vamsi/scripts/du-rs/du-rs/target/release/du-rs")
            .arg("-b")
            .arg("test_env")
            .output()
            .expect("Failed to execute process");

        let stdout = String::from_utf8_lossy(&output.stdout);

        let expected_output = "\
6          test_env/test_dir2
0          test_env/test1
17         test_env
";

        assert_eq!(stdout.trim(), expected_output.trim());
    }
    #[test]
    fn test_du_b_a() {
        setup_test_environment();

        let output = Command::new("/home/vamsi/scripts/du-rs/du-rs/target/release/du-rs")
            .arg("-b")
            .arg("-a")
            .arg("test_env")
            .output()
            .expect("Failed to execute process");

        let stdout = String::from_utf8_lossy(&output.stdout);

        let expected_output = "\
5          test_env/test2.txt
6          test_env/test.txt
6          test_env/test_dir2/test.txt
6          test_env/test_dir2
0          test_env/test1
17         test_env
";

        assert_eq!(stdout.trim(), expected_output.trim());
    }
    #[test]
    fn test_bk() {
        setup_test_environment();
        let output = Command::new("/home/vamsi/scripts/du-rs/du-rs/target/release/du-rs")
            .arg("-BK")
            .arg("test_env")
            .output()
            .expect("Failed to execute process");
        let stdout = String::from_utf8_lossy(&output.stdout);

        let expected_output = "\
8K         test_env/test_dir2
4K         test_env/test1
24K        test_env
24K        ./
";
        assert_eq!(stdout.trim(), expected_output.trim());
    }
    #[test]
    fn test_bm() {
        setup_test_environment();
        let output = Command::new("/home/vamsi/scripts/du-rs/du-rs/target/release/du-rs")
            .arg("-BM")
            .arg("test_env")
            .output()
            .expect("Failed to execute process");
        let stdout = String::from_utf8_lossy(&output.stdout);

        let expected_output = "\
1M         test_env/test_dir2
1M         test_env/test1
1M         test_env
1M         ./
";
        assert_eq!(stdout.trim(), expected_output.trim());
    }
    #[test]
    fn test_bg() {
        setup_test_environment();
        let output = Command::new("/home/vamsi/scripts/du-rs/du-rs/target/release/du-rs")
            .arg("-BG")
            .arg("test_env")
            .output()
            .expect("Failed to execute process");
        let stdout = String::from_utf8_lossy(&output.stdout);

        let expected_output = "\
1G         test_env/test_dir2
1G         test_env/test1
1G         test_env
1G         ./
";
        assert_eq!(stdout.trim(), expected_output.trim());
    }

    #[test]
    fn test_b1024() {
        setup_test_environment();
        let output = Command::new("/home/vamsi/scripts/du-rs/du-rs/target/release/du-rs")
            .arg("-B1024")
            .arg("test_env")
            .output()
            .expect("Failed to execute process");
        let stdout = String::from_utf8_lossy(&output.stdout);

        let expected_output = "\
8          test_env/test_dir2
4          test_env/test1
24         test_env
24         ./
";
        assert_eq!(stdout.trim(), expected_output.trim());
    }
}
