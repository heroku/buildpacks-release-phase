use std::{
    env,
    io::{BufReader, Write},
    path::Path,
};

use flate2::{read::GzDecoder, Compression, GzBuilder};
use std::fs::File;
use std::io::Seek;
use tar::{Archive, Builder};

/// Tars & compresses contents of the given directory to a .tar.gz file.
pub fn create_archive(
    source_dir: &Path,
    destination: impl AsRef<Path>,
) -> Result<(), std::io::Error> {
    let temp_dir = env::temp_dir();
    let tar_filename = "archive.tar";
    let tar_file: File = File::create(temp_dir.join(tar_filename))?;
    let mut tar_builder = Builder::new(&tar_file);
    tar_builder.follow_symlinks(false);
    // add to root of archive
    tar_builder.append_dir_all("", source_dir)?;
    tar_builder.into_inner()?;
    let tar_buf = BufReader::new(&tar_file);
    let output_file: File = File::create(destination)?;
    let mut gz = GzBuilder::new()
        .filename(tar_filename)
        .write(output_file, Compression::default());
    gz.write_all(tar_buf.buffer())?;
    gz.finish()?;
    Ok(())
}

/// Decompresses and untars a given .tar.gz file to the given directory.
pub fn extract_archive(
    source_file: &Path,
    destination: impl AsRef<Path>,
) -> Result<(), std::io::Error> {
    let source = File::open(source_file)?;
    let mut archive = Archive::new(GzDecoder::new(source));
    archive.unpack(destination)
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use crate::{create_archive, extract_archive};

    #[test]
    fn create_archive_should_output_tar_gz_file() {
        let output_file = "artifact-from-test.zip";
        fs::remove_file(output_file).unwrap_or_default();
        create_archive(Path::new("test/fixtures/static-artifacts"), output_file).unwrap();
        let result_metadata = fs::metadata(output_file).unwrap();
        assert!(result_metadata.is_file());
        fs::remove_file(output_file).unwrap_or_default();
    }

    #[test]
    fn create_archive_should_fail_for_missing_source_dir() {
        let output_file = "artifact-from-test.zip";
        fs::remove_file(output_file).unwrap_or_default();
        create_archive(Path::new("non-existent-path"), output_file)
            .expect_err("should fail for missing source dir");
        fs::remove_file(output_file).unwrap_or_default();
    }

    #[test]
    fn extract_archive_should_output_a_directory() {
        let output_dir = "artifacts-from-test";
        fs::remove_dir_all(output_dir).unwrap_or_default();
        extract_archive(Path::new("test/fixtures/static-artifacts.tgz"), output_dir).unwrap();
        let result_metadata = fs::metadata(output_dir).unwrap();
        assert!(result_metadata.is_dir());
        fs::remove_dir_all(output_dir).unwrap_or_default();
    }

    #[test]
    fn extarct_archive_should_fail_for_missing_source_file() {
        let output_dir = "artifacts-from-test";
        fs::remove_dir_all(output_dir).unwrap_or_default();
        extract_archive(Path::new("non-existent-path"), output_dir)
            .expect_err("should fail for missing source file");
        fs::remove_dir_all(output_dir).unwrap_or_default();
    }
}
