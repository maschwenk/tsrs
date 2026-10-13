use tsrs_core::stringutil;

// stringtestutil.go:9
pub fn dedent(text: &str) -> String {
    let mut lines: Vec<String> = text.split('\n').map(std::string::ToString::to_string).collect();
    // Remove blank lines in the beginning and end
    // and convert all tabs in the beginning of line to spaces
    let mut start_line: Option<usize> = None;
    let mut last_line: usize = 0;
    for (i, element) in lines.iter_mut().enumerate() {
        let mut line = (*element).clone();
        let first_non_white = line
            .char_indices()
            .find(|&(_, r)| !stringutil::is_white_space_like(r as u32))
            .map(|(i, _)| i);
        if let Some(fnw) = first_non_white.filter(|&index| index > 0) {
            line = line[0..fnw].replace('\t', "    ") + &line[fnw..];
            element.clone_from(&line);
        }
        let line = line.trim();
        if !line.is_empty() {
            if start_line.is_none() {
                start_line = Some(i);
            }
            last_line = i;
        }
    }
    let mut lines: Vec<String> = start_line.map_or_else(Vec::new, |start_line| {
        lines[start_line..=last_line].to_vec()
    });
    let mut mapped_lines: Vec<&str> = Vec::with_capacity(lines.len());
    mapped_lines.extend(lines.iter().map(|line| if line.trim().is_empty() { "" } else { line }));
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
