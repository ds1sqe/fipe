pub fn add_pad(str: &str, pad: &str) -> String {
    let mut buffer = String::new();
    buffer += pad;

    for char in str.chars() {
        buffer.push(char);
        if char == '\n' {
            buffer += pad;
        }
    }

    buffer
}
