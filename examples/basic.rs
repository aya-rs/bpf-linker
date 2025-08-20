use std::{collections::HashSet, path::Path};

use bpf_linker::{Cpu, Linker, LinkerInput, LinkerOptions, OptLevel, OutputType};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = Path::new("/path/to/object-or-bitcode");
    let bytes: &[u8] = &[]; // An in memory object/bitcode

    // Configure the linker.
    let options = LinkerOptions {
        target: None,
        cpu: Cpu::Generic,
        cpu_features: String::new(),
        libs: vec![],
        optimize: OptLevel::Default,
        unroll_loops: false,
        ignore_inline_never: false,
        dump_module: None,
        llvm_args: vec![],
        disable_expand_memcpy_in_order: false,
        disable_memory_builtins: false,
        btf: false,
    };

    // Create the linker.
    let linker = Linker::new(options)?;

    // Link into an in-memory buffer.
    let out_buf = linker.link_to_buffer(
        vec![
            LinkerInput::try_from(path)?,
            LinkerInput::from(("my buffer", bytes)),
        ],
        OutputType::Bitcode,
        &HashSet::new(),
    )?;

    // Use the buffer as slice of u8
    let bytes = out_buf.as_slice();
    println!("Linked {} bytes into memory)", bytes.len());

    // Link to a file
    linker.link_to_file(
        vec![
            LinkerInput::try_from(path)?,
            LinkerInput::from(("my buffer", bytes)),
        ],
        Path::new("/path/to/output"),
        OutputType::Object,
        &HashSet::new(),
    )?;

    Ok(())
}
