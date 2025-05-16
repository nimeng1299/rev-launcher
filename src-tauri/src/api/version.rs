use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Version {
    major: u32,
    minor: Option<u32>,
    patch: Option<u32>,
    pre_release: Option<String>,
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.major == other.major && self.minor == other.minor && self.patch == other.patch
    }
}

impl Eq for Version {}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        if self.major != other.major {
            return self.major.cmp(&other.major);
        }
        if self.minor != other.minor {
            if self.minor.is_none() {
                return std::cmp::Ordering::Less;
            }
            if other.minor.is_none() {
                return std::cmp::Ordering::Greater;
            }
            return self.minor.unwrap().cmp(&other.minor.unwrap());
        }
        if self.patch != other.patch {
            if self.patch.is_none() {
                return std::cmp::Ordering::Less;
            }
            if other.patch.is_none() {
                return std::cmp::Ordering::Greater;
            }
            return self.patch.unwrap().cmp(&other.patch.unwrap());
        }
        return std::cmp::Ordering::Equal;
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.major)?;
        if let Some(minor) = self.minor {
            write!(f, ".{}", minor)?;
            if let Some(patch) = self.patch {
                write!(f, ".{}", patch)?;
            }
        }
        if let Some(pre_release) = &self.pre_release {
            write!(f, " {}", pre_release)?;
        }
        Ok(())
    }
}

impl Version {
    pub fn from_string(s: &String, ingore: Option<&Vec<char>>) -> Result<Self, String> {
        let mut index = 0;
        let mut start = false;
        let mut end = false;
        let mut v = vec![];
        let mut end_str = "".to_string();
        for ch in s.chars() {
            if ingore.is_some() && ingore.unwrap().contains(&ch) {
                continue;
            }
            if end {
                end_str.push(ch);
                continue;
            }
            if ch.is_ascii_digit() {
                if !start {
                    start = true;
                    v.push(0_u32);
                }
                v[index] = v[index] * 10 + (ch as u32 - '0' as u32);
            } else {
                if start {
                    index += 1;
                    start = false;
                }
                if index >= 3 {
                    end = true;
                    end_str.push(ch);
                }
            }
        }
        let end_str = end_str.trim();
        let pre_release = if end_str.is_empty() {
            None
        } else {
            Some(end_str.to_string())
        };
        let len = v.len();
        if len == 0 {
            return Err("Invalid version string".to_string());
        }
        let mut ver = Version {
            major: v[0],
            minor: None,
            patch: None,
            pre_release: pre_release,
        };
        if len >= 2 {
            ver.minor = Some(v[1]);
        }
        if len >= 3 {
            ver.patch = Some(v[2]);
        }
        Ok(ver)
    }
}

#[cfg(test)]
mod tests {
    use crate::api::version::Version;

    #[test]
    fn display_test() {
        let version = super::Version {
            major: 1,
            minor: Some(0),
            patch: Some(0),
            pre_release: Some("alpha".to_string()),
        };
        assert_eq!(version.to_string(), "1.0.0 alpha");

        let version = super::Version {
            major: 1,
            minor: Some(0),
            patch: None,
            pre_release: Some("alpha".to_string()),
        };
        assert_eq!(version.to_string(), "1.0 alpha");

        let version = super::Version {
            major: 1,
            minor: None,
            patch: Some(0),
            pre_release: Some("alpha".to_string()),
        };
        assert_eq!(version.to_string(), "1 alpha");

        let version = super::Version {
            major: 1,
            minor: Some(0),
            patch: Some(2),
            pre_release: None,
        };
        assert_eq!(version.to_string(), "1.0.2");
    }

    #[test]
    fn ord_test() {
        let version1 = super::Version {
            major: 1,
            minor: Some(0),
            patch: Some(0),
            pre_release: Some("alpha".to_string()),
        };
        let version2 = super::Version {
            major: 1,
            minor: Some(0),
            patch: Some(0),
            pre_release: None,
        };
        assert!(version1 == version2);

        let version1 = super::Version {
            major: 1,
            minor: Some(0),
            patch: Some(0),
            pre_release: Some("alpha".to_string()),
        };
        let version2 = super::Version {
            major: 1,
            minor: None,
            patch: Some(0),
            pre_release: None,
        };
        assert!(version1 > version2);

        let version1 = super::Version {
            major: 1,
            minor: Some(0),
            patch: Some(0),
            pre_release: Some("alpha".to_string()),
        };
        let version2 = super::Version {
            major: 1,
            minor: Some(0),
            patch: Some(10),
            pre_release: None,
        };
        assert!(version1 < version2);
    }
    #[test]
    fn from_string_test() {
        let version = super::Version {
            major: 1,
            minor: Some(0),
            patch: Some(10),
            pre_release: None,
        };
        let version_str = version.to_string();
        assert!(Version::from_string(&version_str, None).unwrap() == version);

        let version = super::Version {
            major: 1,
            minor: Some(0),
            patch: Some(10),
            pre_release: Some("s".to_string()),
        };
        let version_str = "sad 1 sda0 d10   s".to_string();
        assert!(Version::from_string(&version_str, None).unwrap() == version);

        let version = super::Version {
            major: 1,
            minor: Some(0),
            patch: Some(10),
            pre_release: Some("s".to_string()),
        };
        let version_str = "sad 1 sda0 d10  \"  s".to_string();
        assert!(Version::from_string(&version_str, Some(&vec!['"'])).unwrap() == version);

        let version = super::Version {
            major: 1,
            minor: Some(0),
            patch: Some(10),
            pre_release: Some("\"  s".to_string()),
        };
        let version_str = "sad 1 sda0 d10  \"  s   ".to_string();
        assert!(Version::from_string(&version_str, None).unwrap() == version);

        let version = super::Version {
            major: 17,
            minor: Some(0),
            patch: Some(10),
            pre_release: Some("2024-01-16 LTS".to_string()),
        };
        let version_str = "openjdk version \"17.0.10\" 2024-01-16 LTS".to_string();
        assert!(Version::from_string(&version_str, Some(&vec!['"'])).unwrap() == version);
    }
}
