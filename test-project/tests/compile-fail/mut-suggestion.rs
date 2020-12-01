// Copyright 2015 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#[derive(Copy, Clone)]
struct S;

impl S {
    fn mutate(&mut self) {
    }
}

fn func(arg: S) {
    //~^ HELP consider changing this to be mutable
    arg.mutate(); //~ ERROR cannot borrow `arg` as mutable
                  //~| NOTE cannot borrow as mutable
}

fn main() {
    let local = S;
    //~^ HELP consider changing this to be mutable
    local.mutate(); //~ ERROR cannot borrow `local` as mutable
                    //~| NOTE cannot borrow as mutable
}
