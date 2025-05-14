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
            write!(f, "-{}", pre_release)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn display_test() {
        let version = super::Version {
            major: 1,
            minor: Some(0),
            patch: Some(0),
            pre_release: Some("alpha".to_string()),
        };
        assert_eq!(version.to_string(), "1.0.0-alpha");

        let version = super::Version {
            major: 1,
            minor: Some(0),
            patch: None,
            pre_release: Some("alpha".to_string()),
        };
        assert_eq!(version.to_string(), "1.0-alpha");

        let version = super::Version {
            major: 1,
            minor: None,
            patch: Some(0),
            pre_release: Some("alpha".to_string()),
        };
        assert_eq!(version.to_string(), "1-alpha");

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
}
