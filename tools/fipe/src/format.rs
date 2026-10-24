//! In-place syntax formatting; paths remain OS strings throughout.
use std::{
    ffi::OsString,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub fn run(arguments: &[OsString]) -> u8 {
    let mut check = false;
    let mut options = true;
    let mut paths = Vec::new();
    for argument in arguments {
        if options && argument == "--" {
            options = false;
            continue;
        }
        if options && argument == "--check" && !check {
            check = true;
            continue;
        }
        if argument == "-"
            || (options && argument.as_encoded_bytes().starts_with(b"-"))
        {
            #[expect(
                clippy::unnecessary_debug_formatting,
                reason = "Keep unsupported OS arguments quoted and control bytes escaped."
            )]
            {
                eprintln!("fipe fmt: unsupported option {argument:?}");
            }
            return 2;
        }
        paths.push(PathBuf::from(argument));
    }
    if paths.is_empty() {
        eprintln!("usage: fipe fmt [--check] FILE...");
        return 2;
    }
    let mut status = 0;
    for (index, path) in paths.iter().enumerate() {
        match one(path, check, index) {
            Ok(code) => status = status.max(code),
            Err(error) => {
                eprintln!("{}: {error}", path.display());
                status = 2;
            }
        }
    }
    status
}

fn one(path: &Path, check: bool, index: usize) -> Result<u8, String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.file_type().is_file() {
        return Err(
            "expected a regular file, not a symlink or directory".into()
        );
    }
    let original =
        fs::read_to_string(path).map_err(|error| error.to_string())?;
    let formatted = match fipe::format::format(&original) {
        Ok(formatted) => formatted,
        Err(error) => {
            eprintln!("{}: {error}", path.display());
            return Ok(1);
        }
    };
    if formatted == original {
        return Ok(0);
    }
    if check {
        eprintln!("{}: formatting differs", path.display());
        return Ok(1);
    }
    if metadata.permissions().readonly() {
        return Err("file is read-only".into());
    }
    replace(path, &original, &formatted, metadata.permissions(), index)?;
    Ok(0)
}

fn replace(
    path: &Path,
    original: &str,
    formatted: &str,
    permissions: fs::Permissions,
    index: usize,
) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut selected = None;
    for attempt in 0..32 {
        let temporary = parent.join(format!(
            ".fipe-fmt-{}-{index}-{attempt}.tmp",
            std::process::id()
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => {
                selected = Some((temporary, file));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    let (temporary, mut file) = selected.ok_or_else(|| {
        "cannot create a unique formatting temporary".to_owned()
    })?;
    let result = (|| {
        file.write_all(formatted.as_bytes())?;
        file.set_permissions(permissions)?;
        drop(file);
        if fs::read(path)?.as_slice() != original.as_bytes() {
            return Err(std::io::Error::other("file changed while formatting"));
        }
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn stale_original_keeps_current_file_and_removes_temporary() {
        let dir = std::env::temp_dir()
            .join(format!("fipe-fmt-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("input.fipe");
        std::fs::write(&path, "current bytes").unwrap();
        let permissions = std::fs::metadata(&path).unwrap().permissions();
        let error = super::replace(
            &path,
            "stale bytes",
            "formatted bytes",
            permissions,
            0,
        )
        .unwrap_err();
        assert!(error.contains("changed while formatting"));
        assert_eq!(std::fs::read(&path).unwrap(), b"current bytes");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
