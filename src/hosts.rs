use std::io;
use std::path::{Path, PathBuf};
use std::str;
use std::io::{Write, Read};
use std::fs::{File, OpenOptions};

const HEADER: &'static [u8] = b"### START AUTOFORWARD";
const FOOTER: &'static [u8] = b"### END AUTOFORWARD";

#[cfg(unix)]
pub fn hosts_file() -> &'static Path { Path::new("/etc/hosts") }

#[cfg(unix)]
const LINE_SEPARATOR: &'static [u8] = b"\n";

#[cfg(windows)]
const LINE_SEPARATOR: &'static [u8] = b"\r\n";

#[test]
fn update_hosts_does_not_replace() {
    let hosts = vec!["reddit.com".to_owned()];
    let mut target_hosts = tempfile::NamedTempFile::new().unwrap();
    std::fs::copy(Path::new("testdata/hosts"), &target_hosts.path()).unwrap();
    update_hosts_file(&target_hosts.path(), &hosts).unwrap();
    let original = std::fs::read_to_string(&target_hosts).unwrap();

    update_hosts_file(&target_hosts.path(), &hosts).unwrap();
    update_hosts_file(&target_hosts.path(), &hosts).unwrap();

    let updated = std::fs::read_to_string(&target_hosts).unwrap();

    assert_eq!(original, updated);
}

pub fn update_hosts_file(path: &Path, hosts: &Vec<String>) -> Result<(), io::Error> {
    let input_bytes = std::fs::read(path)?;

    let mut output = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)?;
    let result = insert_or_replace_entries(&input_bytes, &generate_host_entries(hosts));
    output.write_all(&result)?;
    Ok(())
}

#[test]
fn test_replace_entries() {
    let input = r#"# This is a commentæøå¡™£¢∞∞§¶•ª¶§∞¢£🦀
### START AUTOFORWARD
127.0.0.1 speil.nais.preprod.local
### END AUTOFORWARD
127.0.0.1 localhost
"#.as_bytes();
    let hosts = generate_host_entries(&vec!["new.nais.preprod.local".to_owned()]);
    let expected = r#"# This is a commentæøå¡™£¢∞∞§¶•ª¶§∞¢£🦀
### START AUTOFORWARD
127.0.0.1 new.nais.preprod.local
### END AUTOFORWARD
127.0.0.1 localhost
"#;

    assert_eq!(str::from_utf8(insert_or_replace_entries(input, &hosts).as_slice()).unwrap(), expected);
}

#[test]
fn append_entries() {
    let input = r#"# This is a commentæøå¡™£¢∞∞§¶•ª¶§∞¢£🦀
127.0.0.1 localhost
"#.as_bytes();

    let hosts = generate_host_entries(&vec!["new.nais.preprod.local".to_owned()]);

    let expected = r#"# This is a commentæøå¡™£¢∞∞§¶•ª¶§∞¢£🦀
127.0.0.1 localhost

### START AUTOFORWARD
127.0.0.1 new.nais.preprod.local
### END AUTOFORWARD
"#;
    assert_eq!(str::from_utf8(insert_or_replace_entries(input, &hosts).as_slice()).unwrap(), expected);

}

fn insert_or_replace_entries(input: &'_ [u8], replacement: &[u8]) -> Vec<u8> {
    if let Some(start) = input.windows(HEADER.len()).position(|v| v == HEADER) {
        let end = input.windows(FOOTER.len()).position(|v| v == FOOTER)
            .expect("Found header without any footer following");

        let mut result = Vec::with_capacity(start + HEADER.len() + LINE_SEPARATOR.len() + replacement.len() + (input.len() - end));
        result.write(&input[..start]);
        result.write(HEADER);
        result.write(LINE_SEPARATOR);
        result.write(replacement);
        result.write(FOOTER);
        result.write(&input[end + FOOTER.len()..]);
        result
    } else {
        let mut result = Vec::with_capacity((3*LINE_SEPARATOR.len()) + HEADER.len() + FOOTER.len() + input.len());
        result.write(input);
        result.write(LINE_SEPARATOR);
        result.write(HEADER);
        result.write(LINE_SEPARATOR);
        result.write(replacement);
        result.write(FOOTER);
        result.write(LINE_SEPARATOR);

        result
    }
}

fn generate_host_entries(hosts: &Vec<String>) -> Vec<u8> {
    let loopback = b"127.0.0.1";
    let bytes = hosts.into_iter()
        .map(|v| v.as_bytes().len() + loopback.len() + 1 + LINE_SEPARATOR.len())
        .sum();

    let mut result = Vec::with_capacity(bytes);

    for host in hosts {
        result.write(loopback);
        result.write(b" ");
        result.write(host.as_bytes());
        result.write(LINE_SEPARATOR);
    }

    result
}
