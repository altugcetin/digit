use std::borrow::Cow;

pub enum Event<'a> {
    Start { name: &'a str, attrs: &'a str, empty: bool },
    End(&'a str),
    Text(&'a str),
}

pub struct Reader<'a> {
    s: &'a str,
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(s: &'a str) -> Self {
        Self { s, pos: 0 }
    }

    fn skip_to(&mut self, pat: &str) {
        self.pos = self.s[self.pos..].find(pat).map_or(self.s.len(), |i| self.pos + i + pat.len());
    }
}

impl<'a> Iterator for Reader<'a> {
    type Item = Event<'a>;

    fn next(&mut self) -> Option<Event<'a>> {
        loop {
            let rest = &self.s[self.pos..];
            if rest.is_empty() {
                return None;
            }
            if !rest.starts_with('<') {
                let end = rest.find('<').unwrap_or(rest.len());
                self.pos += end;
                return Some(Event::Text(&rest[..end]));
            }
            if rest.starts_with("<!--") {
                self.skip_to("-->");
                continue;
            }
            if rest.starts_with("<![CDATA[") {
                let body = &rest[9..];
                let end = body.find("]]>").unwrap_or(body.len());
                self.pos += 9 + end + 3.min(body.len() - end);
                return Some(Event::Text(&body[..end]));
            }
            if rest.starts_with("<?") {
                self.skip_to("?>");
                continue;
            }
            if rest.starts_with("<!") {
                self.skip_to(">");
                continue;
            }
            let bytes = rest.as_bytes();
            let mut quote = 0u8;
            let mut end = None;
            for (i, &b) in bytes.iter().enumerate().skip(1) {
                if quote != 0 {
                    if b == quote {
                        quote = 0;
                    }
                } else if b == b'"' || b == b'\'' {
                    quote = b;
                } else if b == b'>' {
                    end = Some(i);
                    break;
                }
            }
            let end = end.unwrap_or(rest.len() - 1);
            self.pos += end + 1;
            let inner = &rest[1..end];
            if let Some(name) = inner.strip_prefix('/') {
                return Some(Event::End(name.trim()));
            }
            let empty = inner.ends_with('/');
            let inner = inner.trim_end_matches('/');
            let split = inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len());
            return Some(Event::Start { name: &inner[..split], attrs: &inner[split..], empty });
        }
    }
}

pub fn attr<'a>(attrs: &'a str, key: &str) -> Option<&'a str> {
    let mut rest = attrs;
    while let Some(i) = rest.find(key) {
        let before_ok = i == 0 || rest.as_bytes()[i - 1].is_ascii_whitespace();
        let after = rest[i + key.len()..].trim_start();
        if before_ok {
            if let Some(after) = after.strip_prefix('=') {
                let after = after.trim_start();
                let q = after.chars().next()?;
                if q == '"' || q == '\'' {
                    let body = &after[1..];
                    return body.find(q).map(|e| &body[..e]);
                }
            }
        }
        rest = &rest[i + key.len()..];
    }
    None
}

pub fn unescape(s: &str) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(semi) = tail.find(';').filter(|&e| e <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let ent = &tail[1..semi];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_elements_attributes_and_entities() {
        let doc = r#"<?xml version="1.0"?><a x="1" lang='tr'><!-- c --><b/>D&apos;x &amp; &#65;<![CDATA[<raw>]]></a>"#;
        let ev: Vec<String> = Reader::new(doc)
            .map(|e| match e {
                Event::Start { name, attrs, empty } => format!("S:{name}:{}:{}:{empty}", attr(attrs, "x").unwrap_or("-"), attr(attrs, "lang").unwrap_or("-")),
                Event::End(n) => format!("E:{n}"),
                Event::Text(t) => format!("T:{}", unescape(t)),
            })
            .collect();
        assert_eq!(ev, vec!["S:a:1:tr:false", "S:b:-:-:true", "T:D'x & A", "T:<raw>", "E:a"]);
        assert_eq!(attr(r#" xml:lang="de""#, "lang"), None);
    }
}
