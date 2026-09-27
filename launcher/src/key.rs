static MC_KEY: &str = env!("MC_KEY");

pub fn get_mc_key() -> String {
    if cfg!(not(has_mc_key)) {
        // 这条语句只在非 debug 下执行
        println!("release only");
    }

    decrypt(MC_KEY)
}

pub fn encrypt(input: &str) -> String {
    shift_by_position(input, false)
}

// encrypt 的逆操作
pub fn decrypt(input: &str) -> String {
    shift_by_position(input, true)
}

fn shift_by_position(input: &str, reverse: bool) -> String {
    let mut pos = 0i64;
    input
        .chars()
        .map(|c| {
            if c.is_ascii_uppercase() {
                // 大写字母完全不管：不变、不占位
                return c;
            }
            pos += 1;
            shift_char(c, if reverse { -pos } else { pos })
        })
        .collect()
}

// 把 c 在所属类别（数字/小写）内循环位移 shift 位，类别外的字符原样返回
fn shift_char(c: char, shift: i64) -> char {
    let (base, len) = match c {
        '0'..='9' => (u32::from('0'), 10),
        'a'..='z' => (u32::from('a'), 26),
        _ => return c,
    };
    let offset = (i64::from(c as u32) - i64::from(base) + shift).rem_euclid(i64::from(len));
    char::from_u32(base + offset as u32).expect("类别内位移结果仍在 ASCII 类别内")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_by_string_position() {
        assert_eq!(encrypt("a"), "b"); // 第1位 +1
        assert_eq!(encrypt("a1"), "b3"); // '1' 在整个字符串的第2位，+2
    }

    #[test]
    fn wrap_within_category() {
        assert_eq!(encrypt("z"), "a"); // 小写越过 'z' 回到 'a'
        assert_eq!(encrypt("9"), "0"); // 数字越过 '9' 回到 '0'
    }

    #[test]
    fn other_chars_untouched_but_counted() {
        assert_eq!(encrypt("a-b"), "b-e"); // '-' 不变但占第2位，'b' 在第3位 +3
        assert_eq!(encrypt("a中"), "b中"); // 非 ASCII 字符不变
        assert_eq!(encrypt("aZ1"), "bZ3"); // 大写不变也不占位，'1' 排到第2位 +2
        assert_eq!(encrypt("Z9a"), "Z0c"); // 开头的大写跳过，'9' 是第1位、'a' 是第2位
    }

    #[test]
    fn decrypt_reverses_encrypt() {
        let s = "333_dr  是否abcXYZ0189-_.:中";
        assert_eq!(decrypt(&encrypt(s)), s);
    }
}

