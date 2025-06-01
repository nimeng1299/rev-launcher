use serde::de;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone)]
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

impl Serialize for Version {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_struct("Version", 1)?;
        s.serialize_field("value", &self.to_string())?;
        s.end()
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &["value"];
        deserializer.deserialize_struct("Version", FIELDS, VersionVisitor)
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

struct VersionVisitor;

impl<'de> serde::de::Visitor<'de> for VersionVisitor {
    type Value = Version;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a version string")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        let mut value_opt: Option<String> = None;

        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "value" => {
                    if value_opt.is_some() {
                        return Err(de::Error::duplicate_field("value"));
                    }
                    value_opt = Some(map.next_value()?);
                }
                // 忽略任何未知字段
                _ => {
                    let _ = map.next_value::<de::IgnoredAny>()?;
                }
            }
        }
        // 检查字段是否存在
        let value_str = value_opt.ok_or_else(|| de::Error::missing_field("value"))?;

        // 调用 from_string，将错误 (semver::Error) 转换成 serde 的 Error
        match Version::from_string(&value_str, None) {
            Ok(ver) => Ok(ver),
            Err(e) => Err(de::Error::custom(format!(
                "failed to parse Version from string `{}`: {}",
                value_str, e
            ))),
        }
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
    #[test]
    fn de_test() {
        let version = super::Version {
            major: 17,
            minor: Some(0),
            patch: Some(10),
            pre_release: Some("2024-01-16 LTS".to_string()),
        };
        let json = serde_json::to_string(&version).unwrap();
        let deserialized: super::Version = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, version);
    }
}
