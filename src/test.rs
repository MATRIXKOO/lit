use {Instance, Config, Error};

use exec::tool;

use regex::Regex;
use super::Matcher;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

lazy_static! {
    static ref DIRECTIVE_REGEX: Regex = Regex::new("([A-Z-]+):(.*)").unwrap();
}

#[derive(Clone,Debug,PartialEq,Eq)]
pub struct Test
{
    pub path: PathBuf,
    pub directives: Vec<Directive>,
}

#[derive(Clone,Debug,PartialEq,Eq)]
pub struct Directive
{
    pub command: Command,
    pub line: u32,
}

#[derive(Clone,Debug)]
pub enum Command
{
    /// Run an external tool.
    Run(tool::Invocation),
    /// Verify that the output text matches an expression.
    Check(Matcher),
    /// Verify that the very next output line matches an expression.
    CheckNext(Matcher),
    /// Mark the test as supposed to fail.
    XFail,
}

#[derive(Debug)]
pub enum TestResultKind
{
    Pass,
    UnexpectedPass,
    Error(Error),
    Fail {
        message: String,
        stderr: Option<String>,
    },
    ExpectedFailure,
    Skip,
}

#[derive(Debug)]
pub struct TestResult
{
    pub path: PathBuf,
    pub kind: TestResultKind,
}

#[derive(Clone,Debug,PartialEq,Eq)]
pub struct Context
{
    pub exec_search_dirs: Vec<String>,
    pub tests: Vec<Test>,
}

#[derive(Debug)]
pub struct Results
{
    test_results: Vec<TestResult>,
}

impl Test
{
    pub fn parse<P,I>(path: P, chars: I) -> Result<Self,String>
        where P: AsRef<Path>, I: Iterator<Item=char> {
        let mut directives = Vec::new();
        let test_body: String = chars.collect();

        let path = path.as_ref().to_owned();

        for (line_idx, line) in test_body.lines().enumerate() {
            let line_number = line_idx + 1;

            match Directive::maybe_parse(line, line_number as _) {
                Some(Ok(directive)) => directives.push(directive),
                Some(Err(e)) => {
                    return Err(format!(
                        "could not parse directive: {}", e)
                    );
                },
                None => continue,
            }
        }

        Ok(Test {
            path,
            directives: directives,
        })
    }

    pub fn run(&self, config: &Config) -> TestResult {
        if self.is_empty() {
            return TestResult {
                path: self.path.clone(),
                kind: TestResultKind::Skip,
            }
        }

        for instance in self.instances() {
            let kind = instance.run(self, config);

            match kind {
                TestResultKind::Pass => continue,
                TestResultKind::Skip => {
                    return TestResult {
                        path: self.path.clone(),
                        kind: TestResultKind::Pass,
                    }
                },
                _ => {
                    return TestResult {
                        path: self.path.clone(),
                        kind,
                    }
                },
            }
        }

        TestResult {
            path: self.path.clone(),
            kind: TestResultKind::Pass,
        }
    }

    pub fn instances(&self) -> Vec<Instance> {
        self.directives.iter().flat_map(|directive| {
            if let Command::Run(ref invocation) = directive.command {
                Some(Instance::new(invocation.clone()))
            } else {
                None
            }
        }).collect()
    }

    /// Extra test-specific variables.
    pub fn variables(&self) -> HashMap<String, String> {
        let mut v = HashMap::new();
        v.insert("file".to_owned(), self.path.to_str().unwrap().to_owned());
        v
    }

    pub fn is_empty(&self) -> bool {
        self.directives.is_empty()
    }
}

impl Directive
{
    pub fn new(command: Command, line: u32) -> Self {
        Directive {
            command: command,
            line: line,
        }
    }

    /// Checks if a strint is a directive.
    pub fn is_directive(string: &str) -> bool {
        DIRECTIVE_REGEX.is_match(string)
    }

    pub fn maybe_parse(string: &str, line: u32) -> Option<Result<Self,String>> {
        if !DIRECTIVE_REGEX.is_match(string) { return None; }

        let captures = DIRECTIVE_REGEX.captures(string).unwrap();
        let command_str = captures.get(1).unwrap().as_str().trim();
        let after_command_str = captures.get(2).unwrap().as_str().trim();

        match command_str {
            // FIXME: better message if we have 'RUN :'
            "RUN" => {
                let inner_words = after_command_str.split_whitespace();
                let invocation = match tool::Invocation::parse(inner_words) {
                    Ok(i) => i,
                    Err(e) => return Some(Err(e)),
                };

                Some(Ok(Directive::new(Command::Run(invocation), line)))
            },
            "CHECK" => {
                let matcher = Matcher::parse(after_command_str);
                Some(Ok(Directive::new(Command::Check(matcher), line)))
            },
            "CHECK-NEXT" => {
                let matcher = Matcher::parse(after_command_str);
                Some(Ok(Directive::new(Command::CheckNext(matcher), line)))
            },
            "XFAIL" => {
                Some(Ok(Directive::new(Command::XFail, line)))
            },
            _ => {
                Some(Err(format!("command '{}' not known", command_str)))
            },
        }
    }
}

impl Context
{
    pub fn new() -> Self {
        Context {
            exec_search_dirs: Vec::new(),
            tests: Vec::new(),
        }
    }

    pub fn test(mut self, test: Test) -> Self {
        self.tests.push(test);
        self
    }

    pub fn run(&self, config: &Config) -> Results {
        let test_results = self.tests.iter().map(|test| {
            test.run(config)
        }).collect();

        Results {
            test_results: test_results,
        }
    }

    pub fn add_search_dir(&mut self, dir: String) {
        self.exec_search_dirs.push(dir);
    }
}

impl Results
{
    pub fn test_results(&self) -> ::std::slice::Iter<TestResult> {
        self.test_results.iter()
    }

    pub fn iter(&self) -> ::std::slice::Iter<TestResult> {
        self.test_results()
    }
}

impl PartialEq for Command {
    fn eq(&self, other: &Command) -> bool {
        match *self {
            Command::Run(ref a) => if let Command::Run(ref b) = *other { a == b } else { false },
            Command::Check(ref a) => if let Command::Check(ref b) = *other { a.to_string() == b.to_string() } else { false },
            Command::CheckNext(ref a) => if let Command::CheckNext(ref b) = *other { a.to_string() == b.to_string() } else { false },
            Command::XFail => *other == Command::XFail,
        }
    }
}

impl Eq for Command { }

#[cfg(test)]
mod test {
    use super::*;

    fn parse(line: &str) -> Result<Directive, String> {
        Directive::maybe_parse(line, 0).unwrap()
    }

    #[test]
    fn can_parse_run() {
        let _d = parse("; RUN: foo").unwrap();
    }
}

