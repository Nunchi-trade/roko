#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliArgs {
    pub name: Option<String>,
    pub count: Option<u32>,
    pub verbose: bool,
}

impl CliArgs {
    pub fn parse_from<I, S>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut name = None;
        let mut count = None;
        let mut verbose = false;

        let mut iter = args.into_iter().map(Into::into).peekable();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--name" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "--name requires a value".to_string())?;
                    name = Some(value);
                }
                "--count" => {
                    let value = iter
                        .next()
                        .ok_or_else(|| "--count requires a value".to_string())?;
                    count = Some(
                        value
                            .parse::<u32>()
                            .map_err(|_| format!("invalid value for --count: {value}"))?,
                    );
                }
                "--verbose" => {
                    verbose = true;
                }
                _ if arg.starts_with('-') => {
                    return Err(format!("unrecognized flag: {arg}"));
                }
                _ => {
                    // Ignore positional arguments for now.
                }
            }
        }

        Ok(Self { name, count, verbose })
    }

    pub fn parse() -> Result<Self, String> {
        Self::parse_from(std::env::args().skip(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flags() {
        let parsed = CliArgs::parse_from(["--name", "Alice", "--count", "3", "--verbose"])
            .unwrap();
        assert_eq!(
            parsed,
            CliArgs {
                name: Some("Alice".into()),
                count: Some(3),
                verbose: true,
            }
        );
    }

    #[test]
    fn rejects_missing_values() {
        assert!(CliArgs::parse_from(["--name"]).is_err());
        assert!(CliArgs::parse_from(["--count"]).is_err());
    }
}
