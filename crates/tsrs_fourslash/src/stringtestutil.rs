use tsrs_core::stringutil;

// stringtestutil.go:9
pub fn dedent(text: &str) -> String {
    let mut lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
    // Remove blank lines in the beginning and end
    // and convert all tabs in the beginning of line to spaces
    let mut start_line: i32 = -1;
    let mut last_line: usize = 0;
    for i in 0..lines.len() {
        let mut line = lines[i].clone();
        let first_non_white = line.char_indices().find(|&(_, r)| !stringutil::is_white_space_like(r)).map(|(i, _)| i as i32).unwrap_or(-1);
        if first_non_white > 0 {
            let fnw = first_non_white as usize;
            line = line[0..fnw].replace('\t', "    ") + &line[fnw..];
            lines[i] = line.clone();
        }
        let line = line.trim();
        if !line.is_empty() {
            if start_line == -1 {
                start_line = i as i32;
            }
            last_line = i;
        }
    }
    let mut lines: Vec<String> = lines[start_line as usize..last_line + 1].to_vec();
    let mut mapped_lines: Vec<&str> = Vec::with_capacity(lines.len());
    for line in &lines {
        if line.trim().is_empty() {
            mapped_lines.push("");
        } else {
            mapped_lines.push(line);
        }
    }
    let indentation = stringutil::guess_indentation(&mapped_lines);
    if indentation > 0 {
        for line in lines.iter_mut() {
            if line.len() > indentation {
                *line = line[indentation..].to_string();
            } else {
                *line = String::new();
            }
        }
    }
    lines.join("\n")
}
