// pretty-compare-only
// pretty-mode:expanded
// pp-exact:macro.pp

macro_rules! square {
    ($x:expr) => {
        $x * $x
    };
}

fn f() -> i8 {
    square!(5)
}
