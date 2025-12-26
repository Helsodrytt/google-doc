use std::{
    rc::Rc,
    time::{Duration},
};

use percent_encoding::{ utf8_percent_encode};
use reqwest::{
    self, Url,
    blocking::{Body, Client},
    header::{HeaderMap, HeaderValue},
};

use crate::utils;
use crate::{
    error::*,
    utils::{DEFAULT_ENCODE_SET, time_now},
};


pub struct GoogleDoc {
    url: Rc<str>,
    client: Client,

    session_id: Rc<str>,
    doc_id: Rc<str>,
    docs_smv: Rc<str>,
    user_id: Rc<str>,
    big_sid: Option<Rc<str>>,

    content_cache: String,
    closed: bool,

    push_req_count: usize,
    event_req_count: usize,
    rev: usize,
}

impl GoogleDoc {
    pub fn new(url: &str) -> Result<Self, DocError> {
        let mut h_map = HeaderMap::new();
        h_map.append("Host", HeaderValue::from_static("docs.google.com"));
        h_map.append(
            "sec-ch-ua-platform",
            HeaderValue::from_static("\"Windows\""),
        );
        h_map.append(
            "sec-ch-ua",
            HeaderValue::from_static(
                r#""Chromium";v="142", "Google Chrome";v="142", "Not_A Brand";v="99""#,
            ),
        );

        let client = Client::builder()
            // .proxy(Proxy::https("127.0.0.1:7125").unwrap())
            .timeout(Duration::from_secs(60))
            // .time(Duration::from_secs(3))
            .default_headers(h_map)
            .build()
            .unwrap();

        let response = client.get(url).send()?.text().unwrap();

        let session_id = find_substring_between(&response, "_createKixApplication('", "',")
            .ok_or(DocError::ParseError)?;
        let doc_id =
            find_substring_between(&response, "'docid': '", "'").ok_or(DocError::ParseError)?;
        let docs_smv =
            find_substring_between(&response, "\"docs-smv\":", ",").ok_or(DocError::ParseError)?;
        let starting_rev =
            find_substring_between(&response, "DOCS_warmStartDocumentLoader.startLoad( ", ".")
                .ok_or(DocError::ParseError)?;
        let user_id =
            find_substring_between(&response, "'oui': '", "'").ok_or(DocError::ParseError)?;

        let content = find_substring_between(
            &response,
            r#"DOCS_modelChunk = {"chunk":[{"ty":"is","ibi":1,"s":""#,
            r#""},"#,
        )
        .unwrap_or_default();

        let result: GoogleDoc = Self {
            url: Rc::from(url),
            client,

            session_id: Rc::from(session_id),
            rev: starting_rev.parse().unwrap(),
            doc_id: Rc::from(doc_id),
            docs_smv: Rc::from(docs_smv),
            user_id: Rc::from(user_id),
            big_sid: None,

            content_cache: string_parser(content),
            closed: false,

            push_req_count: 0,
            event_req_count: 0,
        };

        return Ok(result);
    }

    fn send_command<T: Into<Body>>(&mut self, req_body: T) -> Result<(), DocError> {
        if self.closed {
            return Err(DocError::ClosedDocUsage);
        }

        let post_url = Url::parse_with_params(
            &("https://docs.google.com/document/d/".to_string() + &self.doc_id + &"/save?"),
            &[
                ("id", self.doc_id.as_ref()),
                ("sid", self.session_id.as_ref()),
                ("vc", "1"),
                ("c", "1"),
                ("w", "1"),
                ("flr", "0"),
                ("smv", self.docs_smv.as_ref()),
                ("smb", self.gen_smb().as_ref()),
                ("includes_info_params", "true"),
                ("cros_files", "false"),
                ("tab", "t.0"),
            ],
        )
        .map_err(|_| DocError::ParseError)?;

        let resp = self
            .client
            .post(post_url)
            .header(
                "Content-Type",
                "application/x-www-form-urlencoded;charset=UTF-8",
            )
            .body(req_body)
            .send()?
            .error_for_status()?;

        self.push_req_count += 1;
        self.rev += 1;

        return Ok(());
    }

    fn gen_zx(&self) -> String {
        let mut arr = ['0'; 12];

        for e in &mut arr {
            *e = rand::random_range(97..122) as u8 as char;
        }
        return arr.iter().collect();
    }

    fn gen_smb(&self) -> String {
        format!("[{}, oAM=]", self.docs_smv)
    }

    fn check_big_sid(&mut self) -> Result<(), DocError> {
        if self.big_sid.is_some() {
            return Ok(());
        }
        let req = Url::parse_with_params(
            &("https://docs.google.com/document/d/".to_string() + &self.doc_id + "/bind?"),
            &[
                ("id", self.doc_id.as_ref()),
                ("sid", self.session_id.as_ref()),
                ("includes_info_params", "true"),
                ("cros_files", "false"),
                ("VER", "8"),
                ("tab", "t.0"),
                ("lsq", "-1"),
                ("u", self.user_id.as_ref()),
                ("vc", "1"),
                ("c", "1"),
                ("w", "1"),
                ("flr", "0"),
                ("gsi", ""),
                ("smv", self.docs_smv.as_ref()),
                ("smb", &self.gen_smb()),
                ("cimpl", "0"),
                ("t", "1"),
                ("zx", self.gen_zx().as_ref()),
            ],
        )
        .map_err(|_| DocError::ParseError)?;

        let response = self
            .client
            .post(req)
            .header(
                "Content-Type",
                "application/x-www-form-urlencoded;charset=UTF-8",
            )
            .body("count=0")
            .send()?
            .text()
            .unwrap();

        let big_id = Rc::from(
            find_substring_between(&response, r#""c",""#, r#"""#).ok_or(DocError::ParseError)?,
        );

        self.big_sid = Some(big_id);

        return Ok(());
    }

    pub fn insert<T: AsRef<str>>(&mut self, string: T, position: usize) -> Result<(), DocError> {
        let body =  build_req_body(&[
            ("rev", (self.rev).to_string().as_ref()),
            ("bundles",utf8_percent_encode(
                &format!("[{{\"commands\":[{{\"ty\":\"is\",\"ibi\":{},\"s\":\"{}\"}}],\"sid\":\"{}\",\"reqId\":{}}}]",
                    position,
                    string.as_ref(),
                    self.session_id,
                    self.push_req_count
                ), DEFAULT_ENCODE_SET).to_string().as_ref())
        ]);

        self.send_command(body)?;

        if position > self.content_cache.len() + 1 {
            return Err(DocError::BrokenCache);
        }
        let pos = self
            .content_cache
            .char_indices()
            .nth(position - 1)
            .map(|(i, _)| i)
            .unwrap_or(self.content_cache.len());

        self.content_cache.insert_str(pos, string.as_ref());

        return Ok(());
    }

    pub fn delete(&mut self, start_pos: usize, end_pos: usize) -> Result<(), DocError> {
        let body = build_req_body(&[
            ("rev", (self.rev).to_string().as_ref()),
            (
                "bundles",
                utf8_percent_encode(
                    &format!(
                        r#"[{{"commands":[{{"ty":"ds","si":{},"ei":{}}}],"sid":"{}","reqId":{}}}]"#,
                        start_pos, end_pos, self.session_id, self.push_req_count,
                    ),
                    utils::DEFAULT_ENCODE_SET,
                )
                .to_string()
                .as_ref(),
            ),
        ]);

        self.send_command(body)?;

        let starting_pos = self
            .content_cache
            .char_indices()
            .nth(start_pos - 1)
            .map(|(i, _)| i)
            .unwrap_or(self.content_cache.len());

        let ending_pos = self
            .content_cache
            .char_indices()
            .nth(end_pos)
            .map(|(i, _)| i)
            .unwrap_or(self.content_cache.len());

        self.content_cache.drain(starting_pos..ending_pos);
        return Ok(());
    }

    pub fn sync(&mut self) -> Result<(), DocError> {
        if self.closed {
            return Err(DocError::ClosedDocUsage);
        }

        self.check_big_sid()?;
        let big_sid = unsafe { self.big_sid.as_ref().unwrap_unchecked() }.as_ref();

        let base_url = Url::parse_with_params(
            &("https://docs.google.com/document/d/".to_string() + &self.doc_id + "/bind?"),
            &[
                ("id", self.doc_id.as_ref()),
                ("sid", self.session_id.as_ref()),
                ("includes_info_params", "true"),
                ("cros_files", "false"),
                ("VER", "8"),
                ("tab", "t.0"),
                ("isq", time_now().to_string().as_ref()),
                ("u", self.user_id.as_ref()),
                ("vc", "1"),
                ("c", "1"),
                ("w", "1"),
                ("flr", "0"),
                ("gsi", ""),
                ("smv", self.docs_smv.as_ref()),
                ("smb", &format!("[{}, oAM=]", self.docs_smv)),
                ("cimpl", "0"),
                ("RID", "rpc"),
                ("SID", big_sid),
                ("CI", "1"),
                ("TYPE", "xmlhttp"),
                ("t", "1"),
            ],
        )
        .map_err(|_| DocError::ParseError)?;
        // loop {
            let mut url = base_url.clone();
            url.query_pairs_mut()
                .append_pair("AID", self.event_req_count.to_string().as_str())
                .append_pair("zx", self.gen_zx().as_ref());
            let resp = self.client.get(url).send().map_err(|e| DocError::from(e));

            if let Err(DocError::Timeout) = resp {
                return Ok(());
            }
            let text = resp?.text()?;

            if text.len() < 25 && text.find("noop").is_some() {
                return Ok(());
            }
            self.event_req_count += 1;
            self.parse_event_response(text)?;
        // }

        return Ok(());
    }

    ///logic may break after encoutering ```"``` in upload request
    fn parse_event_response(&mut self, text: String) -> Result<(), DocError> {
        const TY: &str = r#"ty":""#;

        self.event_req_count = find_max_aid(&text) as usize + 1;
        let mut window = &text[0..];
        while let Some(idx) = window.find(r#""cem":{"as":["#) {
            self.rev = find_substring_between(&window[idx..], ",", "]")
                .ok_or(DocError::ParseError)?
                .parse()
                .map_err(|e| DocError::ParseError)?;
            window = &window[idx + r#""cem":{"as":["#.len()..]
        }
        let mut window = &text[0..];
        loop {
            if let Some(idx) = window.find(TY) {
                window = &window[idx + TY.len()..];

                let command = &window[0..window.find(r#"""#).unwrap()];

                match command {
                    "is" => {
                        let pos: usize = find_substring_between(window, r#""ibi":"#, ",")
                            .ok_or(DocError::ParseError)?
                            .parse()
                            .map_err(|_| DocError::ParseError)?;

                        let starting_idx =
                            window.find(r#""s":""#).ok_or(DocError::ParseError)? + r#""s":""#.len();
                        let subwindow = &window[starting_idx..];
                        let ending_idx = subwindow.find("\"").ok_or(DocError::ParseError)?;

                        let string = string_parser(slice_at_char_boundaries(
                            window,
                            starting_idx,
                            starting_idx + ending_idx,
                        ));

                        if pos as isize - 1 < 0 || pos - 1 > self.content_cache.len() {
                            return Err(DocError::ParseError);
                        }
                        let pos = self
                            .content_cache
                            .char_indices()
                            .nth(pos - 1)
                            .map(|(i, _)| i)
                            .unwrap_or(self.content_cache.len());

                        self.content_cache.insert_str(pos, &string);

                    }
                    "ds" => {
                        let starting_pos: usize = find_substring_between(window, r#""si":"#, ",")
                            .ok_or(DocError::ParseError)?
                            .parse()
                            .map_err(|e| DocError::ParseError)?;
                        let starting_pos = self
                            .content_cache
                            .char_indices()
                            .nth(starting_pos - 1)
                            .map(|(i, _)| i)
                            .unwrap_or(self.content_cache.len());

                        let ending_pos: usize = find_substring_between(window, r#""ei":"#, "}")
                            .ok_or(DocError::ParseError)?
                            .parse()
                            .map_err(|e| DocError::ParseError)?;

                        let ending_pos = self
                            .content_cache
                            .char_indices()
                            .nth(ending_pos)
                            .map(|(i, _)| i)
                            .unwrap_or(self.content_cache.len());

                        if starting_pos as isize - 1 > ending_pos as isize {
                            return Err(DocError::ParseError);
                        }
                        self.content_cache.drain(starting_pos..ending_pos);

                    }
                    _ => {}
                }
            } else {
                break;
            }
        }
        return Ok(());
    }

    pub fn get_content(&self) -> &String {
        return &self.content_cache;
    }

    pub fn close(&mut self) -> Result<(), DocError> {
        if self.closed {
            return Err(DocError::ClosedDocUsage);
        }

        let req = Url::parse_with_params(
            &("https://docs.google.com/document/d/".to_string() + &self.doc_id + "/leave?"),
            &[
                ("id", self.doc_id.as_ref()),
                ("sid", self.session_id.as_ref()),
                ("vc", "1"),
                ("c", "1"),
                ("w", "1"),
                ("flr", "0"),
                ("smv", self.docs_smv.as_ref()),
                ("smb", &self.gen_smb()),
            ],
        )
        .unwrap();
        self.client.get(req).send()?;
        self.closed = true;
        return Ok(());
    }
}

fn find_substring_between<'a, 'b, 'c>(
    text: &'a str,
    starting_sub: &'b str,
    ending_sub: &'c str,
) -> Option<&'a str> {
    let starting_idx = text.find(starting_sub)? + starting_sub.len();
    let ending_idx = text[starting_idx..].find(ending_sub)?;

    let result = &text[starting_idx..starting_idx + ending_idx];

    return Some(result);
}

fn build_req_body<T: AsRef<str>>(params: &[(T, T)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", k.as_ref(), v.as_ref()))
        .collect::<Vec<String>>()
        .join("&")
}

fn slice_at_char_boundaries(s: &str, start: usize, end: usize) -> &str {
    let start_boundary = s
        .char_indices()
        .find(|&(i, _)| i >= start)
        .map(|(i, _)| i)
        .unwrap_or(s.len());

    let end_boundary = s
        .char_indices()
        .find(|&(i, _)| i >= end)
        .map(|(i, _)| i)
        .unwrap_or(s.len());

    &s[start_boundary..end_boundary]
}

fn string_parser(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        let next_c = chars.peek();
        if next_c.is_none() {
            result.push(c);
            break;
        }
        let next_c = unsafe { next_c.unwrap_unchecked() };

        match (c, next_c) {
            ('\\', '\\') => {
                chars.next();
                result.push('\\');
            }

            ('\\', 'n') => {
                chars.next();
                result.push('\n');
            }

            ('\\', 't') => {
                chars.next();
                result.push('\t');
            }

            ('\\', '"') => {
                chars.next();
                result.push('"');
            }

            (_, _) => {
                result.push(c);
            }
        }
    }

    return result;
}

fn find_max_aid(text: &str) -> usize {
    let mut max_num: usize = 0;
    let mut count = 0isize;
    let mut iterator = text.chars().into_iter();
    let mut rstr = false;

    loop {
        let c = iterator.next();
        if c.is_none() {
            break;
        }
        let c = c.unwrap();

        match c {
            ']' => {
                if rstr {
                    continue;
                }
                count -= 1;
            }
            '[' => {
                if rstr {
                    continue;
                }
                count += 1;
                if count == 2 {
                    let mut string = String::new();
                    loop {
                        let char = iterator.next().unwrap();
                        if char == ',' {
                            max_num = string.parse().unwrap();
                            break;
                        }
                        string.push(char);
                    }
                }
            }
            '"' => {
                rstr = !rstr;
            }
            _ => {}
        }
    }

    max_num
}
