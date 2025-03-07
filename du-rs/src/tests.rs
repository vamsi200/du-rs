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

    fn cleanup_test_environment() {
        let _ = fs::remove_dir_all("test_env");
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
4.0K       test_env/test1
4.0K       test_env/test_dir2/test.txt
8.0K       test_env/test_dir2
4.0K       test_env/test2.txt
4.0K       test_env/test.txt
24K       test_env
";

        assert_eq!(stdout.trim(), expected_output.trim());

        cleanup_test_environment();
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
4K       test_env/test1
8       test_env/test_dir2
24       test_env
";

        assert_eq!(stdout.trim(), expected_output.trim());

        cleanup_test_environment();
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
4       test_env/test1
4       test_env/test_dir2/test.txt
8       test_env/test_dir2
4       test_env/test2.txt
4       test_env/test.txt
20       test_env
";

        assert_eq!(stdout.trim(), expected_output.trim());

        cleanup_test_environment();
    }
}
