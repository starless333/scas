pub fn error (line : usize, message : String) {
        report (line, "", message);
}

fn report (line : usize, whre : String, message : String) {
        eprintln! ("[line {line} ] Error {whre}: {message}");
}
