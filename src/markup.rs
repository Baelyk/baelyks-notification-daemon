use std::path::PathBuf;

use log::warn;
use nom::{
    branch::{alt, permutation},
    bytes::{tag, take_until, take_until1},
    character::complete::multispace0,
    combinator::eof,
    error::ParseError,
    multi::many_till,
    sequence::{delimited, pair, separated_pair, terminated},
    IResult, Parser,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkupTag {
    Bold(Vec<MarkupTag>),
    Italic(Vec<MarkupTag>),
    Underline(Vec<MarkupTag>),
    Hyperlink {
        href: String,
        children: Vec<MarkupTag>,
    },
    Image {
        src: PathBuf,
        alt: String,
    },
    Text(String),
}

/// A combinator that takes a parser `inner` and produces a parser that also consumes both leading and
/// trailing whitespace, returning the output of `inner`.
fn ws<'a, O, E: ParseError<&'a str>, F>(inner: F) -> impl Parser<&'a str, Output = O, Error = E>
where
    F: Parser<&'a str, Output = O, Error = E>,
{
    delimited(multispace0, inner, multispace0)
}

fn parse_markup_tag_field<'a, E: ParseError<&'a str>>(
    field: &'a str,
) -> impl Parser<&'a str, Output = &'a str, Error = E> {
    let field_name = separated_pair(tag(field), ws(tag("=")), tag("\""));
    ws(delimited(field_name, take_until1("\""), tag("\"")))
}

fn parse_markup_image(input: &str) -> IResult<&str, MarkupTag> {
    let (remainder, (src, alt)) = delimited(
        tag("<img"),
        permutation((parse_markup_tag_field("src"), parse_markup_tag_field("alt"))),
        tag("/>"),
    )
    .parse(input)?;
    Ok((
        remainder,
        MarkupTag::Image {
            src: src.into(),
            alt: alt.into(),
        },
    ))
}

fn parse_markup_hyperlink(input: &str) -> IResult<&str, MarkupTag> {
    let (remainder, (href, contents)) = pair(
        delimited(tag("<a"), parse_markup_tag_field("href"), tag(">")),
        terminated(take_until("</a>"), tag("</a>")),
    )
    .parse(input)?;

    let (_, children) = parse_markup(contents)?;

    Ok((
        remainder,
        MarkupTag::Hyperlink {
            href: href.into(),
            children,
        },
    ))
}

fn parse_markup_tag(input: &str) -> IResult<&str, MarkupTag> {
    let (remainder, tag_name) = delimited(tag("<"), take_until1(">"), tag(">")).parse(input)?;
    let end_tag = format!("</{}", tag_name);
    let (remainder, contents) = take_until(end_tag.as_str()).parse(remainder)?;
    let (remainder, _) = delimited(tag("</"), take_until(">"), tag(">")).parse(remainder)?;

    let (_, children) = parse_markup(contents)?;

    let markup_tag = match tag_name.chars().next().unwrap() {
        'b' => MarkupTag::Bold(children),
        'u' => MarkupTag::Underline(children),
        'i' => MarkupTag::Italic(children),
        _ => {
            warn!("Unknown tag name {}", tag_name);
            MarkupTag::Text(contents.into())
        }
    };

    Ok((remainder, markup_tag))
}

fn parse_markup_text(input: &str) -> IResult<&str, MarkupTag> {
    let (rem, text) = take_until::<&str, &str, nom::error::Error<&str>>("<")
        .parse(input)
        .unwrap_or(("", input));
    let text = MarkupTag::Text(text.to_string());

    Ok((rem, text))
}

fn parse_markup(input: &str) -> IResult<&str, Vec<MarkupTag>> {
    let (rem, (tags, _)) = many_till(
        alt((
            parse_markup_image,
            parse_markup_hyperlink,
            parse_markup_tag,
            parse_markup_text,
        )),
        eof,
    )
    .parse(input)?;
    assert!(rem.is_empty());
    Ok((rem, tags))
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RichTextSpanStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RichTextSpan {
    pub style: RichTextSpanStyle,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum UngroupedBodyElement {
    Span(RichTextSpan),
    Image { src: PathBuf, alt: String },
}

fn flatten(tree: Vec<MarkupTag>) -> Vec<UngroupedBodyElement> {
    flatten_traverser(
        tree,
        RichTextSpanStyle {
            bold: false,
            italic: false,
            underline: false,
        },
    )
}

fn flatten_traverser(tree: Vec<MarkupTag>, style: RichTextSpanStyle) -> Vec<UngroupedBodyElement> {
    tree.into_iter()
        .flat_map(|tag| match tag {
            MarkupTag::Text(text) => {
                vec![UngroupedBodyElement::Span(RichTextSpan { style, text })]
            }
            MarkupTag::Bold(children) => flatten_traverser(
                children,
                RichTextSpanStyle {
                    bold: true,
                    ..style
                },
            ),
            MarkupTag::Italic(children) => flatten_traverser(
                children,
                RichTextSpanStyle {
                    italic: true,
                    ..style
                },
            ),
            MarkupTag::Underline(children) => flatten_traverser(
                children,
                RichTextSpanStyle {
                    underline: true,
                    ..style
                },
            ),
            MarkupTag::Hyperlink { href: _, children } => {
                flatten_traverser(children, RichTextSpanStyle { ..style })
            }
            MarkupTag::Image { src, alt } => vec![UngroupedBodyElement::Image { src, alt }],
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BodyElement {
    RichText(Vec<RichTextSpan>),
    Image { src: PathBuf, alt: String },
}

fn group(elements: Vec<UngroupedBodyElement>) -> Vec<BodyElement> {
    let mut grouped = vec![];
    let mut elements = elements.into_iter();
    let mut group = vec![];

    loop {
        match elements.next() {
            Some(UngroupedBodyElement::Span(span)) => group.push(span),
            Some(UngroupedBodyElement::Image { src, alt }) => {
                if !group.is_empty() {
                    grouped.push(BodyElement::RichText(group));
                }
                group = vec![];
                grouped.push(BodyElement::Image {
                    src: src.clone(),
                    alt: alt.clone(),
                });
            }
            None => {
                if !group.is_empty() {
                    grouped.push(BodyElement::RichText(group));
                }
                break;
            }
        }
    }

    grouped
}

pub fn markup(text: String) -> Vec<BodyElement> {
    match parse_markup(&text) {
        Ok((_, parsed)) => group(flatten(parsed)),
        Err(error) => {
            warn!("Error parsing body: {error}");
            vec![BodyElement::RichText(vec![RichTextSpan {
                style: RichTextSpanStyle {
                    bold: false,
                    italic: false,
                    underline: false,
                },
                text,
            }])]
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_other() {
        let text = r#"<asdf>Hello!</asdf>"#;
        let (_, parsed) = parse_markup(&text).unwrap();
        assert_eq!(parsed, vec![MarkupTag::Text("Hello!".to_string())])
    }

    #[test]
    fn test_text() {
        let text = r#"some text"#;
        let (_, parsed) = parse_markup(&text).unwrap();
        assert_eq!(parsed, vec![MarkupTag::Text("some text".to_string())])
    }

    #[test]
    fn test_markup() {
        let text = r#"<b>Bold</b><i>Italic</i><u>Underline</u>"#;
        let (_, parsed) = parse_markup(&text).unwrap();
        assert_eq!(
            parsed,
            vec![
                MarkupTag::Bold(vec![MarkupTag::Text("Bold".into())]),
                MarkupTag::Italic(vec![MarkupTag::Text("Italic".into())]),
                MarkupTag::Underline(vec![MarkupTag::Text("Underline".into())])
            ]
        )
    }

    #[test]
    fn test_nesting() {
        let text = r#"<b>Some <i>bold and italic</i> text</b>"#;
        let (_, parsed) = parse_markup(&text).unwrap();
        assert_eq!(
            parsed,
            vec![MarkupTag::Bold(vec![
                MarkupTag::Text("Some ".into()),
                MarkupTag::Italic(vec![MarkupTag::Text("bold and italic".into())]),
                MarkupTag::Text(" text".into())
            ]),]
        );
        let flattened = flatten(parsed);
        assert_eq!(
            flattened,
            vec![
                UngroupedBodyElement::Span(RichTextSpan {
                    style: RichTextSpanStyle {
                        bold: true,
                        italic: false,
                        underline: false,
                    },
                    text: "Some ".into(),
                }),
                UngroupedBodyElement::Span(RichTextSpan {
                    style: RichTextSpanStyle {
                        bold: true,
                        italic: true,
                        underline: false,
                    },
                    text: "bold and italic".into(),
                }),
                UngroupedBodyElement::Span(RichTextSpan {
                    style: RichTextSpanStyle {
                        bold: true,
                        italic: false,
                        underline: false,
                    },
                    text: " text".into(),
                })
            ]
        );
        let grouped = group(flattened);
        assert_eq!(
            grouped,
            vec![BodyElement::RichText(vec![
                RichTextSpan {
                    style: RichTextSpanStyle {
                        bold: true,
                        italic: false,
                        underline: false,
                    },
                    text: "Some ".into(),
                },
                RichTextSpan {
                    style: RichTextSpanStyle {
                        bold: true,
                        italic: true,
                        underline: false,
                    },
                    text: "bold and italic".into(),
                },
                RichTextSpan {
                    style: RichTextSpanStyle {
                        bold: true,
                        italic: false,
                        underline: false,
                    },
                    text: " text".into(),
                }
            ])]
        );
    }

    #[test]
    fn test_hyperlink() {
        let text = r#"<a href="example.com">Link text</a>"#;
        let (_, parsed) = parse_markup(&text).unwrap();
        assert_eq!(
            parsed,
            vec![MarkupTag::Hyperlink {
                href: "example.com".to_string(),
                children: vec![MarkupTag::Text("Link text".to_string())]
            }]
        )
    }

    #[test]
    fn test_image() {
        let text_1 = r#"<img src="/path/to/image" alt="Alternative text"/>"#;
        let text_2 = r#"<img alt="Alternative text" src="/path/to/image"/>"#;
        let (_, parsed) = parse_markup(&text_1).unwrap();
        assert_eq!(
            parsed,
            vec![MarkupTag::Image {
                src: PathBuf::from("/path/to/image"),
                alt: "Alternative text".to_string(),
            }]
        );
        let (_, parsed) = parse_markup(&text_2).unwrap();
        assert_eq!(
            parsed,
            vec![MarkupTag::Image {
                src: PathBuf::from("/path/to/image"),
                alt: "Alternative text".to_string(),
            }]
        );
        let flattened = flatten(parsed);
        assert_eq!(
            flattened,
            vec![UngroupedBodyElement::Image {
                src: PathBuf::from("/path/to/image"),
                alt: "Alternative text".to_string(),
            }]
        );
        let grouped = group(flattened);
        assert_eq!(
            grouped,
            vec![BodyElement::Image {
                src: PathBuf::from("/path/to/image"),
                alt: "Alternative text".to_string(),
            }]
        );
    }
}
