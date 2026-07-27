pub fn error (line : usize, message : &str) {
        report (line, "", message);
}

fn report (line : usize, whre : &str, message : &str) {
        eprintln! ("[line {line} ] Error {whre}: {message}");
}
