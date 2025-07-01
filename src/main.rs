use std::{char, env::args, fs::{self, File}, io::{Read, Stdout, Write}, path::PathBuf, process::ExitCode};

use libc::termios as Termios;

struct RawModeGuard {termios: Termios}
impl Drop for RawModeGuard {
    fn drop(&mut self) {
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &self.termios); };
    }
}

fn enable_raw_mode() -> RawModeGuard {
    let mut termios: Termios = unsafe { std::mem::zeroed::<Termios>() };
    unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut termios); };
    let original_termios: Termios = termios.clone();

    termios.c_iflag &= !(libc::BRKINT | libc::ICRNL | libc::INPCK | libc::ISTRIP | libc::IXON);
    termios.c_oflag &= !(libc::OPOST);
    termios.c_cflag |= libc::CS8;
    termios.c_lflag &= !(libc::ECHO | libc::ICANON | libc::IEXTEN | libc::ISIG);

    unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &termios); };
    RawModeGuard { termios: original_termios }
}

fn read_utf8_or_escape() -> (Option<char>, Option<String>) {
    let mut stdin = std::io::stdin();
    let mut buf: [u8; 4] = [0, 0, 0, 0];
    stdin.read_exact(&mut buf[0..1]).expect("Failed to read from STDIN");

    if buf[0] == 0x1B {
        let mut str = "\x1b".to_string();

        let mut byte = [0];
        loop {
            stdin.read_exact(&mut byte).expect("Failed to read from STDIN");
            str = str.to_string() + String::from_utf8(vec![byte[0]]).unwrap().as_str();

            if byte[0] >= 64 && byte[0] < 127 && byte[0] != 91 {
                return (None, Some(str));
            }
        }

    }

    let num_bytes = if buf[0] < 0x80 {
        1
    } else if (buf[0] & 0xE0) == 0xC0 {
        2
    } else if (buf[0] & 0xF0) == 0xE0 {
        3
    } else if (buf[0] & 0xF8) == 0xF0 {
        4
    } else {
        return (Some('\u{FFFD}'), None);
    };

    if num_bytes > 1 {
        stdin.read_exact(&mut buf[1..num_bytes]).expect("Failed to read from STDIN");
    }

    let str = std::str::from_utf8(&buf[0..num_bytes]).unwrap_or("\u{FFFD}");
    (Some(str.chars().next().unwrap_or('\u{FFFD}')), None)
}

fn write(file: PathBuf, buffer: &Vec<String>) {
    let final_str = buffer.join("\n");

    fs::write(file, final_str);
}

fn get_screen_size() -> Option<(usize, usize)> {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let res = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };

    if res == 0 {
        Some((ws.ws_col as usize, ws.ws_row as usize))
    } else {
        None
    }
}

fn clear_screen(out: &mut Stdout) {
    write!(out, "\x1bc\x1b[H").expect("failed to write to STDOUT")
}

fn set_cursor_pos(out: &mut Stdout, row: usize, column: usize) {
    write!(out, "\x1b[{};{}H", row + 1, column + 1).expect("Failed to write to STDOUT");
}

fn draw_buffer(buffer: &Vec<String>, start_line: usize, line: usize, column: usize, width: usize, height: usize) {
    let mut out = std::io::stdout();
    clear_screen(&mut out);

    let mut lin = 0;
    for current_line in 0..height.min(buffer.len()) {
        set_cursor_pos(&mut out, lin, 0);
        lin += 1;

        let curr_str = buffer[current_line + start_line].clone();

        if curr_str.len() < width {
            write!(out, "{}", curr_str).expect("Failed to write to STDOUT");
        } else if line == current_line {
            write!(out, "{}", curr_str.chars().skip(curr_str.len() - width).take(width).collect::<String>()).expect("Failed to write to STDOUT");
        } else {
            write!(out, "{}", curr_str.chars().take(width).collect::<String>()).expect("Failed to write to STDOUT");
        }

        write!(out, "\x1b[K").expect("Failed to write to STDOUT");
    }

    set_cursor_pos(&mut out, line - start_line, column.min(width));
    out.flush().expect("Failed to write to STDOUT");
}

fn main() -> ExitCode {
    let mut buffer: Vec<String> = vec![];
    let mut line = 0;
    let mut start_line = 0;
    let mut column = 0;

    let path = args().skip(1).collect::<String>();
    let pathbuf = PathBuf::from(path.clone());

    if !pathbuf.is_file() {
        eprintln!("You must specify a file!");
        return ExitCode::FAILURE;
    }

    for file_line in fs::read_to_string(pathbuf.clone()).expect("Unable to read file").split("\n") {
        buffer.push(file_line.to_string());
    }

    let _guard = enable_raw_mode();

    let (w_, h_) = get_screen_size().expect("Unable to get terminal size");
    draw_buffer(&buffer, 0, line, column, w_, h_);

    loop {
        let (char, escape) = read_utf8_or_escape();
        let esc = escape.unwrap_or("".to_string());

        match char {
            Some(char_) => match char_ {
                '\x7F' => { // Backspace
                    let line_content = buffer.get_mut(line).unwrap();
                    if column > 0 {
                        line_content.remove(line_content.char_indices().nth(column - 1).map_or(0, |(byte, _)| byte));
                        column -= 1;
                    } else if buffer.len() > 1 {
                        column = buffer[line - 1].len();
                        buffer[line - 1] = buffer[line - 1].clone() + buffer.remove(line).to_string().as_str();
                        line -= 1;
                    }
                },
                '\n' => { // Enter sometimes
                    line += 1;
                    column = 0;
                    buffer.insert(line, "".to_string());
                }
                '\r' => { // Enter other times
                    line += 1;
                    column = 0;
                    buffer.insert(line, "".to_string());
                }
                '\x11' => { // ctrl+q
                    write(pathbuf.clone(), &buffer);
                    break;
                }
                '\x13' => write(pathbuf.clone(), &buffer), // ctrl+s
                _ => {
                    if !char_.is_control() {
                        let line_ = buffer.get_mut(line).unwrap();
                        let byte_index = line_.char_indices().nth(column).map(|(idx, _)| idx).unwrap_or(line_.len());
                        line_.insert(byte_index, char_);
                        column += 1;
                    }
                }
            },
            None => match esc.as_str() {
                "\x1b[A" => {
                    line = line.saturating_sub(1).max(0);
                },
                "\x1b[B" => {
                    line = (line + 1).min(buffer.len() - 1);
                },
                "\x1b[C" => {
                    column = (column + 1).min(buffer[line].len());
                },
                "\x1b[D" => {
                    column = column.saturating_sub(1).max(0);
                },
                _ => {
                    // Uncomment this to figure out escape sequences for things like function keys
                    // panic!("Unhandled escape sequence: {:?}", esc);
                }
            }
        }

        let (width, height) = get_screen_size().unwrap_or((1, 1));

        if line < start_line + 8 {
            start_line = line.saturating_sub(8);
        }

        if line >= start_line + height - 8 {
            start_line = line - (height - 8) + 1;
        }

        start_line = start_line.max(0);
        if buffer.len() > height {
            start_line = start_line.min(buffer.len() - height);
        } else {
            start_line = 0;
        }

        draw_buffer(&buffer, start_line, line, column, width, height);
    }

    ExitCode::SUCCESS
}

