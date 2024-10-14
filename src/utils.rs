use std::fs::File;
use std::io::Write;
use std::path::Path;

pub fn write_string_to_file<P: AsRef<Path>>(content: &str, file_path: P) -> std::io::Result<()> {
    let mut file = File::create(file_path)?;
    file.write_all(content.as_bytes())?;
    Ok(())
}
