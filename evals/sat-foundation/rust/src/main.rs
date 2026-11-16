use std::io::{self, BufRead, Write};
fn main() {
    let mut output = io::BufWriter::new(io::stdout().lock());
    for input in io::stdin().lock().lines() {
        let input = input.expect("read CNF");
        serde_json::to_writer(&mut output, &idealis_sat_eval::dispatch(input.as_bytes()))
            .expect("write result");
        output.write_all(b"\n").expect("write newline");
        output.flush().expect("flush output");
    }
}
