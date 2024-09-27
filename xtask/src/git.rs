use std::{ffi::OsString, os::unix::ffi::OsStringExt, process::Command};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitError {
    #[error("could not find a git repository")]
    RepositoryNotFound,
}

pub fn top_directory() -> Result<OsString, GitError> {
    let workdir = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output();
    match workdir {
        Ok(output) if output.status.success() => {
            Ok(OsString::from_vec(
                // Remove the trailing `\n` character.
                output.stdout[..output.stdout.len() - 1].to_vec(),
            ))
        }
        Ok(_) => Err(GitError::RepositoryNotFound),
        Err(_) => Err(GitError::RepositoryNotFound),
    }
}
