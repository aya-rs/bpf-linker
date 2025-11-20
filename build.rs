use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    io::{self, Write as _},
    os::unix::ffi::OsStrExt as _,
    path::Path,
    process::Command,
};

use anyhow::{Context as _, anyhow};
use object::{Object as _, ObjectSymbol as _, read::archive::ArchiveFile};

macro_rules! print_bytes {
    ($($bytes:expr),* $(,)?) => {
        {
            let result = (|| -> anyhow::Result<()> {
                $(
                    io::stdout().write_all($bytes).with_context(|| {
                        format!(
                            "failed to write bytes to stdout: {}",
                            OsStr::from_bytes($bytes).display()
                        )
                    })?;
                )*
                io::stdout().flush().with_context(|| "failed to flush stdout")?;
                Ok(())
            })();

            result
        }
    };
}

/// Points cargo to static library names of LLVM and dependencies needed by
/// LLVM.
///
/// That includes figuring out which libraries LLVM depends on:
///
/// - standard C++ library (GNU stdc++ or LLVM libc++),
/// - zlib (optional),
/// - zstd (optional).
///
/// It's necessary, because static libraries have no equivalent of `DT_NEEDED`
/// entries. They come with undefined symbols that must be filled in by other
/// libraries at link time. Since static archives do not explicitly express
/// which additional libraries are required, we have to determine that set
/// ourselves using the undefined symbols, and instruct Cargo to link them.
fn link_llvm_static(llvm_lib_dir: &Path) -> anyhow::Result<()> {
    // LLVM often creates multiple symlinks, e.g. LLVM.so and LLVM-21.so, both
    // pointing to the same target.
    // To avoid duplication, track unique targets by inode, so identical
    // symlinks can be skipped.

    // Link the library files found inside the directory.
    let dir_entries = fs::read_dir(llvm_lib_dir)
        .with_context(|| format!("failed to read directory {}", llvm_lib_dir.display()))?;
    for entry in dir_entries {
        let entry = entry.with_context(|| {
            format!(
                "failed to read entry of the directory {}",
                llvm_lib_dir.display()
            )
        })?;
        let file_name = entry.file_name();
        let file_name = file_name.as_bytes();
        let Some(trimmed) = file_name
            .strip_prefix(b"libLLVM")
            .and_then(|name| name.strip_suffix(b".a"))
        else {
            continue;
        };

        print_bytes!(b"cargo:rustc-link-lib=static=LLVM", trimmed, b"\n")?;
    }

    // Static libraries have no metadata indicating a dependency on other
    // libraries. Given that zlib and zstd might or might be not enabled in
    // different LLVM builds, check whether libLLVMSupport references their
    // symbols.
    let (mut needs_zlib, mut needs_zstd) = (false, false);
    let llvm_support = llvm_lib_dir.join("libLLVMSupport.a");
    let data = fs::read(&llvm_support)
        .with_context(|| format!("failed to read library {}", llvm_support.display()))?;
    let archive = ArchiveFile::parse(data.as_slice())
        .with_context(|| format!("failed to parse library archive {}", llvm_support.display()))?;
    'outer: for member in archive.members() {
        let member = member.with_context(|| {
            format!(
                "failed to process the member of library archive {}",
                llvm_support.display()
            )
        })?;
        let member_data = member.data(data.as_slice()).with_context(|| {
            format!(
                "failed to read data of static library archive member {}",
                OsStr::from_bytes(member.name()).display()
            )
        })?;
        let obj = object::File::parse(member_data).with_context(|| {
            format!(
                "failed to parse object file of static library archive member {}",
                OsStr::from_bytes(member.name()).display()
            )
        })?;
        for symbol in obj.symbols() {
            if symbol.is_undefined() {
                let sym_name = symbol.name().with_context(|| {
                    format!(
                        "invalid symbol name in object file {}",
                        OsStr::from_bytes(member.name()).display()
                    )
                })?;
                if sym_name.contains("crc32") {
                    needs_zlib = true;
                } else if sym_name.contains("ZSTD") {
                    needs_zstd = true;
                }
                if needs_zlib && needs_zstd {
                    break 'outer;
                }
            }
        }
    }

    // Figure out which C++ standard library to use.
    const CXXSTDLIB: &str = "CXXSTDLIB";
    let cxxstdlib = env::var_os(CXXSTDLIB).unwrap_or_else(|| {
        if cfg!(target_os = "linux") {
            // Default to GNU libstdc++ on Linux. Can be overwritten through
            // `CXXSTDLIB` variable on distributions using LLVM as default
            // toolchain.
            OsString::from("stdc++")
        } else if cfg!(target_os = "macos") {
            // Default to LLVM libc++ on macOS, where LLVM is the default
            // toolchain.
            OsString::from("c++")
        } else {
            // Fall back to GNU libstdc++ on all other platforms. Again,
            // can be overwritten through `CXXSTDLIB`.
            OsString::from("stdc++")
        }
    });

    let linkage = if cfg!(feature = "llvm-deps-link-static") {
        // Unfortunately, Rust/cargo don't look for static libraries in system
        // directories, like C compilers do, so we had to implement the logic
        // of searching for them ourselves.

        // Use C compiler to retrieve the system library paths.
        let cc = env::var_os("CC").unwrap_or_else(|| OsString::from("cc"));
        let cc_output = Command::new(&cc)
            .arg("-print-search-dirs")
            .output()
            .with_context(|| format!("failed to run `{} -print-search-dirs`", cc.display()))?;
        if !cc_output.status.success() {
            anyhow::bail!(
                "`{} -print-search-dirs` failed with status: {}",
                cc.display(),
                cc_output.status
            );
        }
        let cc_stdout = String::from_utf8(cc_output.stdout).with_context(|| {
            format!(
                "output of `{} -print-search-dirs` is not valid UTF-8",
                cc.display()
            )
        })?;
        let ld_paths = cc_stdout
        .lines()
        .find_map(|line| {
            line.strip_prefix("libraries: =")
        })
        .ok_or_else(|| {
            anyhow!(
                "failed to find library paths in the output of `{} -print-search-dirs`: {cc_stdout}",
                cc.display()
            )
        })?;

        // Find directories with static libraries we're interested in:
        // - C++ standard library
        // - zlib (if needed)
        // - zstd (if needed)
        let mut cxxstdlib_filename = OsString::with_capacity(3 + cxxstdlib.len() + 2);
        cxxstdlib_filename.push("lib");
        cxxstdlib_filename.push(&cxxstdlib);
        cxxstdlib_filename.push(".a");
        let zlib_filename = "libz.a";
        let zstd_filename = "libzstd.a";
        let (mut cxxstdlib_found, mut zlib_found, mut zstd_found) = (false, false, false);
        for ld_path in env::split_paths(ld_paths) {
            let mut found_any = false;
            let cxxstdlib_path = ld_path.join(&cxxstdlib_filename);
            if !cxxstdlib_found
                && cxxstdlib_path.try_exists().with_context(|| {
                    format!("failed to inspect the file {}", cxxstdlib_path.display(),)
                })?
            {
                cxxstdlib_found = true;
                found_any = true;
            }
            let zlib_path = ld_path.join(&zlib_filename);
            if needs_zlib
                && !zlib_found
                && zlib_path.try_exists().with_context(|| {
                    format!("failed to inspect the file {}", zlib_path.display())
                })?
            {
                zlib_found = true;
                found_any = true;
            }
            let zstd_path = ld_path.join(&zstd_filename);
            if needs_zstd
                && !zstd_found
                && zstd_path.try_exists().with_context(|| {
                    format!("failed to inspect the file {}", zstd_path.display())
                })?
            {
                zstd_found = true;
                found_any = true;
            }
            if found_any {
                print_bytes!(
                    b"cargo:rustc-link-search=",
                    ld_path.as_os_str().as_bytes(),
                    b"\n"
                )?;
            }
        }
        if !cxxstdlib_found {
            anyhow::bail!(
                "could not find {} in any of the following directories: {ld_paths}",
                cxxstdlib_filename.display()
            );
        }
        if needs_zlib && !zlib_found {
            anyhow::bail!(
                "could not find {zlib_filename} in any of the following directories: {ld_paths}"
            );
        }
        if needs_zstd && !zstd_found {
            anyhow::bail!(
                "could not find {zstd_filename} in any of the following directories: {ld_paths}"
            );
        }

        "static"
    } else {
        "dylib"
    };

    print_bytes!(
        b"cargo:rustc-link-lib=",
        linkage.as_bytes(),
        b"=",
        cxxstdlib.as_bytes(),
        b"\n"
    )?;
    if needs_zlib {
        println!("cargo:rustc-link-lib={linkage}=z");
    }
    if needs_zstd {
        println!("cargo:rustc-link-lib={linkage}=zstd");
    }

    Ok(())
}

/// Points cargo to shared library name of LLVM.
///
/// Unlike [`link_llvm_static`], it does not require explicit search for
/// dependencies, since shared libraries contain `DT_NEEDED` entries that
/// specify the names of libaries that the dynamic linker should link
/// beforehand.
fn link_llvm_dynamic(llvm_lib_dir: &Path) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    const DYLIB_EXT: &[u8] = b".dylib";
    #[cfg(not(target_os = "macos"))]
    const DYLIB_EXT: &[u8] = b".so";

    let dir_entries = fs::read_dir(llvm_lib_dir).with_context(|| {
        format!(
            "failed to read entry of the directory {}",
            llvm_lib_dir.display()
        )
    })?;
    let mut found = false;
    for entry in dir_entries {
        let entry = entry.with_context(|| {
            format!(
                "failed to read entry of the directory {}",
                llvm_lib_dir.display()
            )
        })?;
        let file_name = entry.file_name();
        let file_name = file_name.as_bytes();
        if let Some(trimmed) = file_name
            .strip_prefix(b"libLLVM")
            .and_then(|name| name.strip_suffix(DYLIB_EXT))
        {
            print_bytes!(b"cargo:rustc-link-lib=dylib=LLVM", trimmed)?;
            found = true;
            break;
        };
    }
    if !found {
        anyhow::bail!(
            "could not find dynamic libLLVM in the directory {}",
            llvm_lib_dir.display()
        );
    }

    Ok(())
}

/// Points cargo to the path containing LLVM libraries and to the LLVM library
/// files.
fn link_llvm() -> anyhow::Result<()> {
    // If `LLVM_PREFIX` variable is not provided, find the directory with LLVM
    // libraries by assuming it's `lib/` inside a prefix where `llvm-config`
    // lives.
    const LLVM_PREFIX: &str = "LLVM_PREFIX";
    const PATH: &str = "PATH";
    let (var_name, paths_os) = env::var_os(LLVM_PREFIX)
        .map(|mut p| {
            p.push("/bin");
            (LLVM_PREFIX, p)
        })
        .or_else(|| env::var_os(PATH).map(|p| (PATH, p)))
        .ok_or_else(|| anyhow!("neither {LLVM_PREFIX} nor {PATH} is set"))?;
    let llvm_config = env::split_paths(&paths_os)
        .map(|dir| {
            let candidate = Path::new(&dir).join("llvm-config");
            candidate.try_exists().map(|exists| (candidate, exists))
        })
        .find_map(|res| match res {
            Ok((candidate, true)) => Some(Ok(candidate)),
            Ok((_, false)) => None,
            Err(e) => Some(Err(e)), // propagate the error
        })
        .transpose()? // convert Option<Result<T>> into Result<Option<T>>
        .with_context(|| format!("could not find llvm-config in {var_name}"))?;
    let llvm_lib_dir = llvm_config
        .parent()
        .and_then(|p| p.parent())
        .ok_or_else(|| {
            anyhow!(
                "llvm-config location has no parent: {}",
                llvm_config.display()
            )
        })?
        .join("lib");
    let llvm_lib_dir = fs::canonicalize(&llvm_lib_dir).with_context(|| {
        format!(
            "failed to canonicalize LLVM lib directory {}",
            llvm_lib_dir.display()
        )
    })?;
    let llvm_lib_dir_str = llvm_lib_dir
        .to_str()
        .ok_or_else(|| anyhow!("path {} is not a valid UTF-8", llvm_lib_dir.display()))?;
    println!("cargo:rustc-link-search={llvm_lib_dir_str}");

    if cfg!(feature = "llvm-link-static") {
        link_llvm_static(&llvm_lib_dir)?;
    } else if cfg!(not(feature = "llvm-link-static")) {
        link_llvm_dynamic(&llvm_lib_dir)?;
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    if cfg!(not(feature = "no-llvm-linking")) {
        link_llvm()?;
    } else if cfg!(all(
        feature = "no-llvm-linking",
        any(
            feature = "llvm-link-static",
            feature = "llvm-deps-link-static"
        )
    )) {
        anyhow::bail!(
            "`no-llvm-linking` and linking features (`deps-link-static` and
`llvm-link-static`) are mutually exclusive"
        );
    }
    Ok(())
}
