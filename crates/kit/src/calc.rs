struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn peek(&mut self) -> Option<u8> {
        while self.s.get(self.i).is_some_and(|c| c.is_ascii_whitespace()) {
            self.i += 1;
        }
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        loop {
            if self.eat(b'+') {
                v += self.term()?;
            } else if self.eat(b'-') {
                v -= self.term()?;
            } else {
                return Some(v);
            }
        }
    }

    fn term(&mut self) -> Option<f64> {
        let mut v = self.unary()?;
        loop {
            match self.peek() {
                Some(b'*' | b'x' | b'\xc3') if self.s[self.i] != b'x' || !self.word_follows() => {
                    if self.s[self.i] == b'\xc3' {
                        if self.s.get(self.i + 1) != Some(&b'\x97') {
                            return Some(v);
                        }
                        self.i += 2;
                    } else {
                        self.i += 1;
                    }
                    v *= self.unary()?;
                }
                Some(b'/') => {
                    self.i += 1;
                    v /= self.unary()?;
                }
                Some(b'%') => {
                    self.i += 1;
                    if matches!(self.peek(), None | Some(b')' | b'+' | b'-' | b'*' | b'/')) {
                        v /= 100.0;
                    } else {
                        v %= self.unary()?;
                    }
                }
                Some(b'(') => v *= self.unary()?,
                Some(c) if c.is_ascii_alphabetic() && self.s[self.i] != b'x' => v *= self.unary()?,
                _ => return Some(v),
            }
        }
    }

    fn word_follows(&self) -> bool {
        self.s.get(self.i + 1).is_some_and(|c| c.is_ascii_alphabetic())
    }

    fn unary(&mut self) -> Option<f64> {
        if self.eat(b'-') {
            return Some(-self.unary()?);
        }
        if self.eat(b'+') {
            return self.unary();
        }
        let base = self.atom()?;
        if self.eat(b'^') {
            return Some(base.powf(self.unary()?));
        }
        Some(base)
    }

    fn atom(&mut self) -> Option<f64> {
        let c = self.peek()?;
        if self.eat(b'(') {
            let v = self.expr()?;
            self.eat(b')');
            return Some(v);
        }
        if c.is_ascii_digit() || c == b'.' || c == b',' {
            let start = self.i;
            while self.s.get(self.i).is_some_and(|c| c.is_ascii_digit() || *c == b'.' || *c == b',' || *c == b'_') {
                self.i += 1;
            }
            if matches!(self.s.get(self.i), Some(b'e' | b'E')) && self.s.get(self.i + 1).is_some_and(|c| c.is_ascii_digit() || *c == b'-' || *c == b'+') {
                self.i += 2;
                while self.s.get(self.i).is_some_and(|c| c.is_ascii_digit()) {
                    self.i += 1;
                }
            }
            let text: String = std::str::from_utf8(&self.s[start..self.i]).ok()?.chars().filter(|c| *c != '_').map(|c| if c == ',' { '.' } else { c }).collect();
            return text.parse().ok();
        }
        if c.is_ascii_alphabetic() {
            let start = self.i;
            while self.s.get(self.i).is_some_and(|c| c.is_ascii_alphanumeric()) {
                self.i += 1;
            }
            let name = std::str::from_utf8(&self.s[start..self.i]).ok()?.to_ascii_lowercase();
            return match name.as_str() {
                "pi" => Some(std::f64::consts::PI),
                "e" => Some(std::f64::consts::E),
                "tau" => Some(std::f64::consts::TAU),
                _ => {
                    let arg = self.unary()?;
                    Some(match name.as_str() {
                        "sqrt" => arg.sqrt(),
                        "cbrt" => arg.cbrt(),
                        "sin" => arg.sin(),
                        "cos" => arg.cos(),
                        "tan" => arg.tan(),
                        "asin" => arg.asin(),
                        "acos" => arg.acos(),
                        "atan" => arg.atan(),
                        "ln" => arg.ln(),
                        "log" => arg.log10(),
                        "log2" => arg.log2(),
                        "exp" => arg.exp(),
                        "abs" => arg.abs(),
                        "round" => arg.round(),
                        "floor" => arg.floor(),
                        "ceil" => arg.ceil(),
                        _ => return None,
                    })
                }
            };
        }
        None
    }
}

pub fn eval(text: &str) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut p = Parser { s: text.as_bytes(), i: 0 };
    let v = p.expr()?;
    (p.peek().is_none() && v.is_finite()).then_some(v)
}

pub fn looks_like_math(text: &str) -> bool {
    let t = text.trim();
    t.chars().any(|c| c.is_ascii_digit()) && (t.chars().any(|c| "+-*/^%×()".contains(c)) || t.starts_with("sqrt") || t.starts_with("pi"))
}

pub fn format(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let s = format!("{:.10}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.len() > 18 { format!("{v:e}") } else { s.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic() {
        assert_eq!(eval("1+2*3"), Some(7.0));
        assert_eq!(eval("(1+2)*3"), Some(9.0));
        assert_eq!(eval("2^10"), Some(1024.0));
        assert_eq!(eval("-3 + 5"), Some(2.0));
        assert_eq!(eval("10 / 4"), Some(2.5));
        assert_eq!(eval("50%"), Some(0.5));
        assert_eq!(eval("200 * 15%"), Some(30.0));
        assert_eq!(eval("7 % 3"), Some(1.0));
        assert_eq!(eval("2(3+4)"), Some(14.0));
        assert_eq!(eval("3 x 4"), Some(12.0));
        assert_eq!(eval("1,5 * 2"), Some(3.0));
        assert_eq!(eval("1_000 * 2"), Some(2000.0));
        assert_eq!(eval("2pi").map(|v| (v * 1000.0).round()), Some(6283.0));
        assert_eq!(eval("sqrt 16"), Some(4.0));
        assert_eq!(eval("sqrt(16)+1"), Some(5.0));
        assert_eq!(eval("1e3"), Some(1000.0));
    }

    #[test]
    fn rejects() {
        assert_eq!(eval("firefox"), None);
        assert_eq!(eval("1/0"), None);
        assert_eq!(eval("2+"), None);
        assert_eq!(eval(""), None);
        assert!(!looks_like_math("firefox"));
        assert!(looks_like_math("12*4"));
    }

    #[test]
    fn formatting() {
        assert_eq!(format(4.0), "4");
        assert_eq!(format(2.5), "2.5");
        assert_eq!(format(1.0 / 3.0), "0.3333333333");
    }
}
