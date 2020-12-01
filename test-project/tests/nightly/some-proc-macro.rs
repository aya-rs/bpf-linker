// run-pass
// aux-build:simple_proc_macro.rs

#![feature(proc_macro_hygiene)]

extern crate simple_proc_macro;
use simple_proc_macro::macro_test;

fn main() {
    println!("hi");
    macro_test!(2);
}

