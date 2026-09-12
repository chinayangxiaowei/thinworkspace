pub(crate) fn parse_path_list(output: Vec<u8>) -> Result<Vec<Vec<u8>>, &'static str> {
    if !output.is_empty() && output.last() != Some(&0) {
        return Err("non-empty Git path output was not NUL-terminated");
    }

    let mut paths = nul_fields(&output)
        .into_iter()
        .map(<[u8]>::to_vec)
        .collect::<Vec<_>>();
    if paths.iter().any(Vec::is_empty) {
        return Err("empty path in Git output");
    }
    sort_paths(&mut paths);
    Ok(paths)
}

fn nul_fields(output: &[u8]) -> Vec<&[u8]> {
    let mut fields = output.split(|byte| *byte == 0).collect::<Vec<_>>();
    if fields.last() == Some(&&[][..]) {
        fields.pop();
    }
    fields
}

fn sort_paths(paths: &mut [Vec<u8>]) {
    paths.sort();
}
