//@ mode: c
//@ run-status: 0

use std::f64::consts::PI;
use std::time::Instant;
use std::time::Duration;
use std::thread;
use std::vec::Vec;
use std::mem::drop;

int main() {
    if (PI < 3.14 || std::f64::consts::PI > 3.15) {
        return 1;
    }

    Instant instant = Instant::now();
    thread::sleep(Duration::from_millis(10));
    Duration dur = instant.elapsed();
    if (dur.as_millis() < 5 || Duration::as_millis(&dur) > 50) {
        return 2;
    }

    return 0;
}
