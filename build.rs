use std::{
    array,
    borrow::Cow,
    collections::{HashMap, VecDeque},
    env,
    ffi::{OsStr, OsString, os_str},
    fmt::{self, Display, Formatter},
    fs,
    io::{self, Write as _},
    iter,
    os::unix::ffi::OsStrExt as _,
    path::{self, Path, PathBuf},
    process::Command,
};

use anyhow::{Context as _, anyhow};
use object::{Object as _, ObjectSymbol as _, read::archive::ArchiveFile};

macro_rules! write_bytes {
    ($dst:expr, $($bytes:expr),* $(,)?) => {
        (|| {
            use std::io::{IoSlice, Error, ErrorKind};
            let mut bufs = [
                $(
                    IoSlice::new($bytes.as_ref()),
                )*
                IoSlice::new(b"\n"),
            ];
            // TODO(https://github.com/rust-lang/rust/issues/70436): use `write_all_vectored` when stable.
            let mut bufs = &mut bufs[..];
            IoSlice::advance_slices(&mut bufs, 0);
            while !bufs.is_empty() {
                match $dst.write_vectored(bufs) {
                    Ok(0) => {
                        return Err(Error::new(ErrorKind::WriteZero, "failed to write whole buffer"));
                    }
                    Ok(n) => IoSlice::advance_slices(&mut bufs, n),
                    Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
                    Err(e) => return Err(e),
                }
            }
            Ok(())
        })().map_err(Into::<anyhow::Error>::into)
    };
}

/// Representation of a library that needs to be linked.
enum Library<'a> {
    /// Library (or multiple libraries separated by commas) that were provided
    /// through an environment variable.
    EnvVar(OsString),
    /// A single library, with name as bytes.
    Single(&'static [u8]),
    /// Multiple libraries, with names as bytes.
    Multiple(&'a [&'static [u8]]),
}

impl Library<'_> {
    /// Detects which standard C++ library to link and creates an appropriate
    /// [`Self`] representation.
    fn cxxstdlib(stdout: &mut io::StdoutLock<'_>) -> anyhow::Result<Self> {
        const CXXSTDLIB: &str = "CXXSTDLIB";
        writeln!(stdout, "cargo:rerun-if-env-changed={CXXSTDLIB}")?;
        Ok(match env::var_os(CXXSTDLIB) {
            Some(cxxstdlib) => Self::EnvVar(cxxstdlib),
            None => {
                if cfg!(target_os = "linux") {
                    // Default to GNU libstdc++ on Linux. Can be overwritten through
                    // `CXXSTDLIB` variable on distributions using LLVM as default
                    // toolchain.
                    Self::Single(b"stdc++")
                } else if cfg!(target_os = "macos") {
                    // Default to LLVM libc++ on macOS, where LLVM is the default
                    // toolchain.
                    if cfg!(feature = "llvm-link-static") {
                        // Static LLVM libc++ has two files - libc++.a and libc++abi.a.
                        Self::Multiple(&[b"c++", b"c++abi"])
                    } else {
                        // Shared LLVM libc++ has one file.
                        Self::Single(b"c++")
                    }
                } else {
                    // Fall back to GNU libstdc++ on all other platforms. Again,
                    // can be overwritten through `CXXSTDLIB`.
                    Self::Single(b"stdc++")
                }
            }
        })
    }

    fn iter(&self) -> impl Iterator<Item = &[u8]> {
        match self {
            Self::EnvVar(p) => LibraryIter::Parsed(p.as_bytes().split(|b| *b == b',')),
            Self::Single(s) => {
                LibraryIter::Single(iter::once(
                    // Coerce `&&[u8]` to `&[u8]`.
                    *s,
                ))
            }
            Self::Multiple(m) => LibraryIter::Multiple(
                m.iter()
                    // Coerce `&&[u8]` to `&[u8]`.
                    .copied(),
            ),
        }
    }

    fn iter_static_filenames(&self) -> impl Iterator<Item = (&[u8], OsString)> {
        self.iter().map(|lib| {
            let mut filename = OsString::from("lib");
            filename.push(OsStr::from_bytes(lib));
            filename.push(".a");
            (lib, filename)
        })
    }
}

impl Display for Library<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::EnvVar(p) => {
                Display::fmt(&p.display(), f)?;
            }
            Self::Single(s) => Display::fmt(&OsStr::from_bytes(s).display(), f)?,
            Self::Multiple(m) => {
                f.write_str("[")?;
                for (i, lib) in m.iter().enumerate() {
                    if i != 0 {
                        write!(f, ", ")?;
                    }
                    Display::fmt(&OsStr::from_bytes(lib).display(), f)?;
                }
                f.write_str("]")?;
            }
        }
        Ok(())
    }
}

enum LibraryIter<P, S, M> {
    Parsed(P),
    Single(S),
    Multiple(M),
}

impl<'a, P, S, M> Iterator for LibraryIter<P, S, M>
where
    P: Iterator<Item = &'a [u8]>,
    S: Iterator<Item = &'a [u8]>,
    M: Iterator<Item = &'a [u8]>,
{
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Parsed(p) => p.next(),
            Self::Single(s) => s.next(),
            Self::Multiple(m) => m.next(),
        }
    }
}

/// Representation of the paths that we search for libraries in.
enum Paths<'a> {
    /// Colon-separated paths parsed from the C compiler.
    LdPaths(&'a OsStr),
    /// A single path.
    Single(PathBuf),
}

impl Paths<'_> {
    fn display(&self) -> PathsDisplay<'_> {
        match self {
            Self::LdPaths(ld_paths) => PathsDisplay::LdPaths(ld_paths.display()),
            Self::Single(path) => PathsDisplay::Single(path.display()),
        }
    }

    fn iter(&self) -> PathsIter<'_> {
        match self {
            Self::LdPaths(ld_paths) => PathsIter::LdPaths(env::split_paths(ld_paths)),
            Self::Single(single) => PathsIter::Single(iter::once(single)),
        }
    }
}

enum PathsDisplay<'a> {
    LdPaths(os_str::Display<'a>),
    Single(path::Display<'a>),
}

impl Display for PathsDisplay<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::LdPaths(display) => display.fmt(f),
            Self::Single(display) => display.fmt(f),
        }
    }
}

enum PathsIter<'a> {
    LdPaths(env::SplitPaths<'a>),
    Single(iter::Once<&'a PathBuf>),
}

impl<'a> Iterator for PathsIter<'a> {
    type Item = Cow<'a, PathBuf>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            PathsIter::LdPaths(iter) => iter.next().map(Cow::Owned),
            PathsIter::Single(iter) => iter.next().map(Cow::Borrowed),
        }
    }
}

/// Represents a located library with its corresponding path.
struct LocatedLibrary {
    library_name: OsString,
    library_path: PathBuf,
}

impl AsRef<Path> for LocatedLibrary {
    fn as_ref(&self) -> &Path {
        &self.library_path
    }
}

/// Validates the set of candidate `paths` for `library`.
///
/// Does nothing if exactly one path is provided.
///
/// Emits a warning (including content hashes) if multiple paths are present.
///
/// Returns an error if `paths` is empty.
fn check_library<S: Display, P: AsRef<Path>>(
    stdout: &mut io::StdoutLock<'_>,
    ld_paths: &Paths<'_>,
    library: S,
    mut paths: VecDeque<P>,
) -> anyhow::Result<P> {
    let path = paths.pop_front().ok_or_else(|| {
        anyhow::anyhow!(
            "could not find {library} in any of the following directories: {}",
            ld_paths.display()
        )
    })?;
    if !paths.is_empty() {
        let mut hashes = HashMap::new();
        const BUF_SIZE: usize = 8 * 1024;
        let mut buffer = [0; BUF_SIZE];
        fn add_path_hash<'a>(
            hashes: &mut HashMap<u64, Vec<&'a Path>>,
            buffer: &mut [u8; BUF_SIZE],
            path: &'a Path,
        ) -> anyhow::Result<()> {
            use std::{hash::Hasher as _, io::Read as _};
            let mut hasher = std::hash::DefaultHasher::new();
            let mut file = fs::File::open(path)
                .with_context(|| format!("failed to open file {}", path.display()))?;
            loop {
                let n = file
                    .read(buffer)
                    .with_context(|| format!("failed to read file {}", path.display()))?;
                if n == 0 {
                    break;
                }
                hasher.write(&buffer[..n]);
            }
            hashes.entry(hasher.finish()).or_default().push(path);
            Ok(())
        }
        add_path_hash(&mut hashes, &mut buffer, path.as_ref())?;
        for path in paths.iter() {
            add_path_hash(&mut hashes, &mut buffer, path.as_ref())?;
        }
        if hashes.len() > 1 {
            write!(
                stdout,
                "cargo:warning={library} was found in multiple locations: "
            )?;
            for (i, (hash, paths)) in hashes.iter().enumerate() {
                if i != 0 {
                    write!(stdout, ", ")?;
                }
                write!(stdout, "[")?;
                for (i, path) in paths.iter().enumerate() {
                    if i != 0 {
                        write!(stdout, ", ")?;
                    }
                    write!(stdout, "{}", path.display())?;
                }
                write!(stdout, "]=0x{hash:x}")?;
            }
            writeln!(stdout)?;
        }
    }
    Ok(path)
}

fn find_and_link_libraries<const N: usize>(
    stdout: &mut io::StdoutLock<'_>,
    ld_paths: Paths<'_>,
    libraries: &[Library<'_>; N],
) -> anyhow::Result<()> {
    // Collecions of all library candidate paths. Might contain duplicates if
    // a library is present in different directories from `ld_paths`.
    let mut library_candidates: [_; N] = array::from_fn(|_| VecDeque::new());
    for ld_path in ld_paths.iter() {
        for (i, library) in libraries.iter().enumerate() {
            for (_, filename) in library.iter_static_filenames() {
                let library_path = ld_path.join(filename);
                if library_path.try_exists().with_context(|| {
                    format!("failed to inspect the file {}", library_path.display())
                })? {
                    library_candidates[i].push_back(library_path);
                }
            }
        }
    }
    // Check whether each library has at least one candidate path. If not,
    // return an error. If a library has more than one candidate, pick the
    // first one and emit a warning.
    // Emit the appropriate Cargo instructions to link the found liraries.
    for (i, library) in libraries.iter().enumerate() {
        let library_candidates = std::mem::take(&mut library_candidates[i]);
        let library_path = check_library(stdout, &ld_paths, library, library_candidates)?;
        let library_dir = library_path
            .parent()
            .expect("`library_path` should have a parent");
        write_bytes!(
            stdout,
            "cargo:rustc-link-search=",
            library_dir.as_os_str().as_bytes()
        )?;
        for library in library.iter() {
            write_bytes!(stdout, "cargo:rustc-link-lib=static=", library)?;
        }
    }
    Ok(())
}

/// Checks whether the given environment variable `env_var` exists and if yes,
/// searches for the given `library` in it and links it.
///
/// Returns a boolean indicating whether the variable was found
fn find_and_link_libary_in_defined_path(
    stdout: &mut io::StdoutLock<'_>,
    library: &Library<'_>,
    env_var: &str,
) -> anyhow::Result<bool> {
    writeln!(stdout, "cargo:rerun-if-env-changed={env_var}")?;
    match env::var_os(env_var) {
        Some(path) => {
            write_bytes!(
                stdout,
                "cargo:rustc-link-search=",
                path.as_os_str().as_bytes(),
            )?;
            for (library, filename) in library.iter_static_filenames() {
                let library_path = Path::new(&path).join(filename);
                if library_path.try_exists().with_context(|| {
                    format!("failed to inspect the file {}", library_path.display())
                })? {
                    write_bytes!(stdout, "cargo:rustc-link-lib=static=", library)?;
                }
            }
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Points cargo to static library names of LLVM and dependencies needed by
/// LLVM.
///
/// That includes figuring out which libraries LLVM depends on:
///
/// - Standard C++ library (GNU stdc++ or LLVM libc++).
/// - zlib (optional).
/// - zstd (optional).
///
/// It's necessary, because static libraries have no equivalent of `DT_NEEDED`
/// entries. They come with undefined symbols that must be filled in by other
/// libraries at link time. Since static archives do not explicitly express
/// which additional libraries are required, we have to determine that set
/// ourselves using the undefined symbols, and instruct Cargo to link them.
fn link_llvm_static(stdout: &mut io::StdoutLock<'_>, llvm_lib_dir: PathBuf) -> anyhow::Result<()> {
    let llvm_support = llvm_lib_dir.join("libLLVMSupport.a");

    // Static libraries have no metadata indicating a dependency on other
    // libraries. Given that zlib and zstd might or might be not enabled in
    // different LLVM builds, check whether libLLVMSupport references their
    // symbols.
    let (mut needs_zlib, mut needs_zstd) = (false, false);
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
                "failed to parse data of static library archive member {} as object file",
                OsStr::from_bytes(member.name()).display()
            )
        })?;
        for symbol in obj.symbols() {
            if symbol.is_undefined() {
                let sym_name = symbol.name().with_context(|| {
                    format!(
                        "failed to retrieve the symbol name in static library archive member {}",
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

    // Link the LLVM library files found inside the directory.
    let dir_entries = fs::read_dir(&llvm_lib_dir)
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

        write_bytes!(stdout, "cargo:rustc-link-lib=static=LLVM", trimmed)?;
    }

    let cxxstdlib = Library::cxxstdlib(stdout)?;
    const ZLIB: &[u8] = b"z";
    let zlib = Library::Single(ZLIB);
    const ZSTD: &[u8] = b"zstd";
    let zstd = Library::Single(ZSTD);

    // Find directories with static libraries we're interested in:
    // - C++ standard library
    // - zlib (if needed)
    // - zstd (if needed)

    // Check whether custom paths were provided. If yes, point the linker to
    // them.
    let cxxstdlib_found =
        find_and_link_libary_in_defined_path(stdout, &cxxstdlib, "CXXSTDLIB_PATH")?;
    let zlib_found =
        needs_zlib && find_and_link_libary_in_defined_path(stdout, &zlib, "ZLIB_PATH")?;
    let zstd_found =
        needs_zstd && find_and_link_libary_in_defined_path(stdout, &zstd, "LIBZSTD_PATH")?;

    if !cxxstdlib_found || (needs_zlib && !zlib_found) || (needs_zstd && !zstd_found) {
        // Unfortunately, Rust/cargo don't look for static libraries in system
        // directories, like C compilers do, so we had to implement the logic
        // of searching for them ourselves.

        // Use C compiler to retrieve the system library paths with
        // `-print-search-dirs` option.
        const CC: &str = "CC";
        const RUSTC_LINKER: &str = "RUSTC_LINKER";
        /// Executes `maybe_cc` with `-print-search-dirs` argument. On success,
        /// returns the output.
        fn print_search_dirs<'a>(
            stdout: &mut io::StdoutLock<'_>,
            var: Option<&str>,
        ) -> anyhow::Result<Option<(Cow<'a, OsStr>, Vec<u8>)>> {
            let maybe_cc = match var {
                Some(var) => {
                    writeln!(stdout, "cargo:rerun-if-env-changed={var}")?;
                    match env::var_os(var).map(Cow::Owned) {
                        Some(var) => var,
                        // If the environment variable is not defined, proceed with
                        // the next candidate.
                        None => return Ok(None),
                    }
                }
                // Use `cc` as the last option. Pretty much all UNIX-like operating
                // systems provide `/usr/bin/cc` as a symlink to the default
                // compiler (either clang or gcc).
                None => Cow::Borrowed(OsStr::from_bytes(b"cc")),
            };
            let mut cmd = Command::new(&maybe_cc);
            let linker_output = cmd
                .arg("-print-search-dirs")
                .output()
                .with_context(|| format!("failed to run {cmd:?}"))?;
            if linker_output.status.success() {
                Ok(Some((maybe_cc, linker_output.stdout)))
            } else {
                // We don't return an error here, instead we log a warning and
                // proceed with trying next candidates.
                // The failure here usually means that `maybe_cc` is a regular
                // linker (e.g. rust-lld, bfd, lld), not a C compiler (e.g.
                // clang, gcc).
                if let Some(var) = var {
                    writeln!(
                        stdout,
                        "cargo:warning=`command `{cmd:?}` (specified by environment variable
{var}: {}) failed: {}",
                        maybe_cc.display(),
                        linker_output.status
                    )?;
                } else {
                    writeln!(
                        stdout,
                        "cargo:warning=`command `{cmd:?}` failed: {}",
                        linker_output.status
                    )?;
                }
                Ok(None)
            }
        }
        let (cc, linker_stdout) = [
            // Try to use the `CC` environment variable, allowing users to
            // overwrite defaults.
            Some(CC),
            // Try to retrieve it from `RUSTC_LINKER` environment
            // variable. It's defined by cargo only if `-C linker` option is
            // provided. Users are not supposed to define this variable on
            // their own.
            // Note that it might be either a C compiler or a regular linker.
            Some(RUSTC_LINKER),
            None,
        ]
        .into_iter()
        .find_map(|var| print_search_dirs(stdout, var).transpose())
        .transpose()?
        .ok_or_else(|| {
            anyhow!(
                "could not find C compiler capable of reporting library search paths,
(with `-print-search-dirs`), consider setting `CC` environment variable pointing
to an appropriate compiler"
            )
        })?;
        let ld_paths = linker_stdout
            .split(|&b| b == b'\n')
            .find_map(|line| line.strip_prefix(b"libraries: ="))
            .ok_or_else(|| {
                anyhow!(
                    "failed to find library paths in the output of `{} -print-search-dirs`: {}",
                    cc.display(),
                    OsStr::from_bytes(&linker_stdout).display()
                )
            })?;
        let ld_paths = OsStr::from_bytes(ld_paths);

        // Find directories with static libraries we're interested in:
        // - C++ standard library
        // - zlib (if needed)
        // - zstd (if needed)
        const ZLIB: &[u8] = b"z";
        const ZSTD: &[u8] = b"zstd";
        find_and_link_libraries(
            stdout,
            Paths::LdPaths(ld_paths),
            &[cxxstdlib, Library::Single(ZLIB), Library::Single(ZSTD)],
        )?;
    }

    Ok(())
}

/// Points cargo to shared library name of LLVM.
///
/// Unlike [`link_llvm_static`], it does not require explicit search for
/// dependencies, since shared libraries contain `DT_NEEDED` entries that
/// specify the names of libaries that the dynamic linker should link
/// beforehand.
fn link_llvm_dynamic(stdout: &mut io::StdoutLock<'_>, llvm_lib_dir: PathBuf) -> anyhow::Result<()> {
    const LIB_LLVM: &[u8] = b"libLLVM";
    #[cfg(target_os = "macos")]
    const DYLIB_EXT: &[u8] = b".dylib";
    #[cfg(not(target_os = "macos"))]
    const DYLIB_EXT: &[u8] = b".so";

    let dir_entries = fs::read_dir(&llvm_lib_dir).with_context(|| {
        format!(
            "failed to read entry of the directory {}",
            llvm_lib_dir.display()
        )
    })?;
    let mut libraries = VecDeque::new();
    for entry in dir_entries {
        let entry = entry.with_context(|| {
            format!(
                "failed to read entry of the directory {}",
                llvm_lib_dir.display()
            )
        })?;

        let mut file_name = entry.file_name().into_encoded_bytes();
        if file_name.starts_with(LIB_LLVM) && file_name.ends_with(DYLIB_EXT) {
            drop(file_name.drain((file_name.len() - DYLIB_EXT.len())..));
            drop(file_name.drain(..LIB_LLVM.len()));
            libraries.push_back(LocatedLibrary {
                // SAFETY: `file_name` originates from `OsString::into_encoded_bytes`.
                // Since then, it was only trimmed.
                library_name: unsafe { OsString::from_encoded_bytes_unchecked(file_name) },
                library_path: entry.path(),
            });
        }
    }
    write_bytes!(
        stdout,
        "cargo:rustc-link-search=",
        llvm_lib_dir.as_os_str().as_bytes()
    )?;
    let library = check_library(
        stdout,
        &Paths::Single(llvm_lib_dir),
        Library::Single(b"LLVM"),
        libraries,
    )?;
    write_bytes!(
        stdout,
        "cargo:rustc-link-lib=dylib=",
        library.library_name.as_bytes(),
    )?;
    Ok(())
}

/// Points cargo to the path containing LLVM libraries and to the LLVM library
/// files.
fn main() -> anyhow::Result<()> {
    let link_fn = if cfg!(feature = "no-llvm-linking") {
        if cfg!(feature = "llvm-link-static") {
            anyhow::bail!("`no-llvm-linking` and `llvm-link-static` are mutually exclusive");
        }
        return Ok(());
    } else if cfg!(feature = "llvm-link-static") {
        link_llvm_static
    } else {
        link_llvm_dynamic
    };

    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    // If `LLVM_PREFIX` variable is not provided, find the directory with LLVM
    // libraries by assuming it's `lib/` inside a prefix where `llvm-config`
    // lives.
    const LLVM_PREFIX: &str = "LLVM_PREFIX";
    const PATH: &str = "PATH";
    writeln!(stdout, "cargo:rerun-if-env-changed={LLVM_PREFIX}")?;
    writeln!(stdout, "cargo:rerun-if-env-changed={PATH}")?;
    let (var_name, paths_os) = env::var_os(LLVM_PREFIX)
        .map(|mut p| {
            p.push("/bin");
            (LLVM_PREFIX, p)
        })
        .or_else(|| env::var_os(PATH).map(|p| (PATH, p)))
        .ok_or_else(|| anyhow!("neither {LLVM_PREFIX} nor {PATH} is set"))?;
    let llvm_config = env::split_paths(&paths_os)
        .find_map(|dir| {
            let candidate = Path::new(&dir).join("llvm-config");
            candidate
                .try_exists()
                .with_context(|| format!("failed to inspect the file {}", candidate.display()))
                .map(|exists| exists.then_some(candidate))
                .transpose()
        })
        .transpose()?
        .ok_or_else(|| {
            anyhow!(
                "could not find llvm-config in directories specified by environment
variable `{var_name}` {}",
                paths_os.display()
            )
        })?;
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
    write_bytes!(
        stdout,
        b"cargo:rustc-link-search=",
        llvm_lib_dir.as_os_str().as_bytes(),
    )?;

    link_fn(&mut stdout, llvm_lib_dir)
}
