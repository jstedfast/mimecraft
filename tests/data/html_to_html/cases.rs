// Adapted from MimeKit's UnitTests/Text/HtmlToHtmlTests.cs.
// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

pub const TRUNCATED_TAGS_ARE_DROPPED: &[(&str, &str)] = &[
    ("foo<img src=x onerror=alert(1)", "foo"),
    ("foo <a href=\"javascript:alert(1)", "foo "),
    ("foo<div class=x ", "foo"),
    ("foo</div", "foo"),
    ("foo</div x", "foo"),
    ("foo<", "foo&lt;"),
    ("foo</", "foo&lt;/"),
];

pub const OUTPUT_HTML_FRAGMENT: &[(&str, &str)] = &[
    (
        "<!DOCTYPE html><html><head><title>t</title><style>p{}</style></head><body><p>x<br/></p></body></html>",
        "<p>x<br/></p>",
    ),
    ("<html><body/><body>x</body></html>", "x"),
    (
        "<head><meta charset=utf-8></head>x</body></html><p>y</p>",
        "x<p>y</p>",
    ),
    ("<p>no structure</p>", "<p>no structure</p>"),
];

pub const OUTPUT_HTML_FRAGMENT_IMPLICITLY_CLOSES_HEAD: &[(&str, &str)] = &[
    ("<head><math></head><script><t>", "<math><script><t>"),
    (
        "<head><title>T</title><div>D</div></head><body>B</body>",
        "<div>D</div>B",
    ),
    (
        "<head><title>T</title>junk<style>s</style></head>B",
        "junk<style>s</style>B",
    ),
    (
        "<head><title>T</title><math><mi>x</mi></head><script><t>",
        "<math><mi>x</mi><script><t>",
    ),
    ("<head> \r\n<title>T</title> </head>B", "B"),
    ("<head><o:p>x</o:p></head>", "<o:p>x</o:p>"),
    ("<head><body>B</body>", "B"),
    ("<head><meta charset=utf-8></body>B", "B"),
    ("<head/><title>T</title>B", "B"),
    ("<head>A</head><head>B</head>", "AB"),
    ("<head><head><title>T</title></head>B</head>", "B"),
    (
        "<p>A</p><head><style>s</style></head>",
        "<p>A</p><style>s</style>",
    ),
    ("A<head><title>T</title></head>", "A<title>T</title>"),
    (
        "<title>T</title><head><title>U</title></head>B",
        "<title>T</title>B",
    ),
    (
        "<head><template><div>D</div></head></template>X</head>B",
        "XB",
    ),
    ("<head><template><math></template></head>B", "B"),
    ("<head><noscript><b>x</b></noscript></head>B", "<b>x</b>B"),
];

pub const MAX_ELEMENT_DEPTH_EXCEEDED: &[(&str, &str)] = &[
    (
        "<div><span><b><i>x<img src=x onerror=alert(1)>&amp;</i></b>",
        "<div><span><b><i>x&lt;img src=x onerror=alert(1)&gt;&amp;amp;&lt;/i&gt;&lt;/b&gt;",
    ),
    (
        "<div><span><svg><style><img src=x onerror=alert(1)></style>",
        "<div><span><svg><style>&lt;img src=x onerror=alert(1)&gt;&lt;/style&gt;",
    ),
    (
        "<div><div><div><div><span><b>x</b><img src=x>",
        "<div><div><div><div><span><b>x</b><img src=\"x\"/>",
    ),
];

pub const NO_SCRIPT_UNWRAP_XSS: &[(&str, &str)] = &[
    (
        "<noscript><style></noscript><img src=x onerror=alert(1)></style></noscript>",
        "<style></noscript><img src=x onerror=alert(1)></style>",
    ),
    (
        "<noscript><!--</noscript><img src=x onerror=alert(1)>--></noscript>",
        "<!--</noscript><img src=x onerror=alert(1)>-->",
    ),
    (
        "<noscript><p title=\"</noscript><img src=x onerror=alert(1)>\"></p></noscript>",
        "<p title=\"&lt;/noscript&gt;&lt;img src=x onerror=alert(1)&gt;\"></p>",
    ),
];

pub const NO_SCRIPT_UNWRAP_IGNORED_END_TAG: &[(&str, &str)] = &[
    ("<span><noscript><svg></span>x", "<span><svg>x"),
    (
        "<span><noscript><svg></span>x</noscript></span>y",
        "<span><svg>x</svg></span>y",
    ),
    ("<span><noscript><math><mi></span>x", "<span><math><mi>x"),
];

pub const FRAMESET_IGNORED_TAGS: &[(&str, bool, &str)] = &[
    (
        "<frameset><textarea><!--</textarea><img src=x onerror=alert(1)>--></frameset>",
        false,
        "<frameset><!--</textarea><img src=x onerror=alert(1)>--></frameset>",
    ),
    (
        "<frameset><textarea><!--</textarea><img src=x onerror=alert(1)>--></frameset>",
        true,
        "<!--</textarea><img src=x onerror=alert(1)>-->",
    ),
    (
        "<frameset><frame src=\"a\"><noframes><b>x</b></noframes><style><frame src=\"b\"></frameset><svg><title>",
        false,
        "<frameset><frame src=\"a\"><noframes><b>x</b></noframes><frame src=\"b\"></frameset>",
    ),
    (
        "<frameset><frame src=\"a\"><noframes><b>x</b></noframes><style><frame src=\"b\"></frameset><svg><title>",
        true,
        "<frame src=\"a\"><noframes><b>x</b></noframes><frame src=\"b\">",
    ),
    (
        "x<frameset><textarea><!--</textarea><img src=x>-->",
        true,
        "x<textarea><!--</textarea>-->",
    ),
];

pub const IGNORED_FRAMESET_START_TAG: &[(&str, bool, &str)] = &[
    ("<body><frameset><d>x</d>", false, "<body><d>x</d>"),
    ("<body><frameset><d>x</d>", true, "<d>x</d>"),
    (
        "<div><template></template></div><frameset><d>x</d>",
        false,
        "<div><template></template></div><d>x</d>",
    ),
    (
        "<div><template></template></div><frameset><d>x</d>",
        true,
        "<div><template></template></div><d>x</d>",
    ),
    (
        "x<frameset><d>y</d></frameset>",
        false,
        "x<d>y</d></frameset>",
    ),
    (
        "<frameset><frame><frameset><frame></frameset></frameset>",
        true,
        "<frameset><frame><frameset><frame></frameset></frameset>",
    ),
];

pub const NO_SCRIPT_SCRIPTING_DISABLED_REMOVE_NO_SCRIPT_TAGS: &[(&str, &str)] = &[
    (
        "<noscript><style></noscript><img src=x onerror=alert(1)></style></noscript>",
        "<style></noscript><img src=x onerror=alert(1)></style>",
    ),
    (
        "<noscript><!--</noscript><img src=x onerror=alert(1)>--></noscript>",
        "<!--</noscript><img src=x onerror=alert(1)>-->",
    ),
    (
        "<noscript><p title=\"</noscript><img src=x onerror=alert(1)>\"></p></noscript>",
        "<p title=\"&lt;/noscript&gt;&lt;img src=x onerror=alert(1)&gt;\"></p>",
    ),
];

pub const OUT_OF_ORDER_END_TAGS: &[(&str, &str)] = &[
    ("<a><b>1</a>2</b>3", "<a><b>1</a>2</b>3"),
    ("<x>1<y>2</x>3</y>4", "<x></x>3</y>4"),
    ("<x>1<X>2</x>3</X>4", "<x></X>4"),
    ("<y><x>1</y>2</x>3<x>4</X>5", "<y><x></x>3<x></X>5"),
    ("<x>1<x>2<x>3</x>4</x>5</x>6", "<x></x>6"),
    ("</x>1<x>2</y>3</x>4</x>5", "</x>1<x></x>4</x>5"),
];

pub const SHORT_BOGUS_DOC_TYPE_KEYWORD: &[(&str, &str)] = &[
    (
        "<!DOCTYPE html </DIV><DIV class=x>y</DIV>",
        "<!DOCTYPE html><DIV class=\"x\">y</DIV>",
    ),
    ("<!DOCTYPE html bog><p>y</p>", "<!DOCTYPE html><p>y</p>"),
];

pub const DOC_TYPE_IDENTIFIER_QUOTES: &[(&str, Option<&str>, &str)] = &[
    ("<!DOCTYPE x SYSTEM 'a\"b'><p>y</p>", None, "a\"b"),
    (
        "<!DOCTYPE x PUBLIC 'a\"b' 'c\"d'><p>y</p>",
        Some("a\"b"),
        "c\"d",
    ),
    ("<!DOCTYPE x PUBLIC \"a'b\" 'c'><p>y</p>", Some("a'b"), "c"),
    ("<!DOCTYPE x SYSTEM 'a'><p>y</p>", None, "a"),
];

pub const IMPLICITLY_CLOSED_FOREIGN_ELEMENTS: &[(&str, bool, &str)] = &[
    (
        "<svg><body><style><img src=x onerror=alert(1)></style>",
        true,
        "<svg></svg><style><img src=x onerror=alert(1)></style>",
    ),
    (
        "<svg><body><style><img src=x onerror=alert(1)></style>",
        false,
        "<svg></svg><body><style><img src=x onerror=alert(1)></style>",
    ),
    (
        "<svg><img src=y><style><img src=x onerror=alert(1)></style>",
        false,
        "<svg></svg><style><img src=x onerror=alert(1)></style>",
    ),
    (
        "<div><svg><g></div><style><img src=x onerror=alert(1)></style>",
        false,
        "<svg><g></g></svg><style><img src=x onerror=alert(1)></style>",
    ),
    (
        "<math><mtext><svg><p></math><b>x</b>",
        false,
        "<math><mtext><svg></svg><p></math><b>x</b>",
    ),
    (
        "<svg><g><g><g></p>x",
        false,
        "<svg><g><g><g></g></g></g></svg></p>x",
    ),
    ("<svg><g></svg>x", false, "<svg><g></g></svg>x"),
    (
        "<mi/r><table><math><mi><e><tr>",
        false,
        "<mi r><table><math><mi><e></e></mi></math><tr>",
    ),
    (
        "<mi><table><math><mi><ne></mi></math><tr><b>x</b>",
        false,
        "<mi><table><math><mi><ne></mi></math></ne></mi></math><tr><b>x</b>",
    ),
    (
        "<table><math><mi><e/>x<table>",
        false,
        "<table><math><mi><e/>x</e></mi></math><table>",
    ),
    (
        "<table><math><mi><e/>x</mi></math><table>",
        false,
        "<table><math><mi><e/>x</mi></math></e></mi></math><table>",
    ),
    ("<svg><noscript><g><p>x", false, "<svg><g></g></svg><p>x"),
];

pub const TAG_CALLBACK_SEES_ALL_TAGS: &[&str] = &[
    "<<img src=x onerror=alert(1)>",
    "<script>x</scrip</script><img src=x onerror=alert(1)>",
    "<style>x</styl</style><img src=x onerror=alert(1)>",
    "<textarea>x</textare</textarea><img src=x onerror=alert(1)>",
    "<!DOC><img src=x onerror=alert(1)>",
    "<![CDAT><img src=x onerror=alert(1)>",
    "<script><!--<script</script><img src=x onerror=alert(1)>",
    "<script><!--</a <script></script><!--</script><img src=x onerror=alert(1)>-->",
    "<script><!--<script></a></script></script><img src=x onerror=alert(1)>",
    "<style/><!--</style><img src=x onerror=alert(1)>-->",
    "<script/><!--</script><img src=x onerror=alert(1)>-->",
    "<textarea/><!--</textarea><img src=x onerror=alert(1)>-->",
    "<![CDATA[ x ><img src=x onerror=alert(1)>]]>",
    "<svg><style><img src=x onerror=alert(1)></style></svg>",
    "<math><style><img src=x onerror=alert(1)></style></math>",
    "<svg><script><img src=x onerror=alert(1)></script></svg>",
    "<div><svg></div><style><!--</style><img src=x onerror=alert(1)>-->",
    "<foo><svg></foo><style><!--</style><img src=x onerror=alert(1)>-->",
    "<svg><p><style><!--</style><img src=x onerror=alert(1)>-->",
    "<form><svg></form><style><img src=x onerror=alert(1)></style>",
    "<div><form></div><form><svg></form><style><img src=x onerror=alert(1)></style>",
    "<svg><![CDATA[]]><img src=x onerror=alert(1)>",
];

pub const SUPPRESS_SELF_CLOSING_RAW_TEXT_ELEMENT: &[(&str, &str)] = &[
    ("<style/>body { color: red; }</style><p>x</p>", "<p>x</p>"),
    ("<script/>alert(1)</script><p>x</p>", "<p>x</p>"),
];

pub const DROPPED_MARKUP_DOES_NOT_FORM_NEW_TAG: &[(&str, bool, &str)] = &[
    ("<</>c>", false, "&lt;c>"),
    (
        "<<!-- x -->img src=x onerror=alert(1)>",
        true,
        "&lt;img src=x onerror=alert(1)>",
    ),
    ("<<!-- x -->img>", false, "&lt;<!-- x -->img>"),
    (
        "a <<b>img onerror=alert(1)>",
        false,
        "a &lt;img onerror=alert(1)>",
    ),
];

pub const BOGUS_COMMENT_ROUND_TRIP: &[(&str, &str)] = &[
    ("</<script x>", "</<script x>"),
    ("</ <img src=x>", "</ <img src=x>"),
    ("</<s", "</<s>"),
    ("<?xml version=\"1.0\"?>", "<?xml version=\"1.0\"?>"),
    ("</?x>", "<?x>"),
    ("<!DOC>", "<!DOC>"),
    ("<!-x>", "<!-x>"),
];

pub const SUPPRESS_INNER_CONTENT_INPUT: &str = "<html xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:o=\"urn:schemas-microsoft-com:office:office\" xmlns:w=\"urn:schemas-microsoft-com:office:word\" xmlns:m=\"http://schemas.microsoft.com/office/2004/12/omml\"xmlns=\"http://www.w3.org/TR/REC-html40\"><head><meta http-equiv=Content-Type content=\"text/html; charset=iso-8859-2\"><meta name=Generator content=\"Microsoft Word 15 (filtered medium)\"><!--[if !mso]><style>v\\:* {behavior:url(#default#VML);}\r\no\\:* {behavior:url(#default#VML);}\r\nw\\:* {behavior:url(#default#VML);}\r\n.shape{behavior:url(#default#VML);}\r\n</style><![endif]--><style><!--\r\n/* Font Definitions */\r\n@font-face\r\n\t{font-family:\"Cambria Math\";\r\n\tpanose-1:2 4 5 3 5 4 6 3 2 4;}\r\n@font-face\r\n\t{font-family:Calibri;\r\n\tpanose-1:2 15 5 2 2 2 4 3 2 4;}\r\n@font-face\r\n\t{font-family:\"Segoe UI\";\r\n\tpanose-1:2 11 5 2 4 2 4 2 2 3;}\r\n@font-face\r\n\t{font-family:Verdana;\r\n\tpanose-1:2 11 6 4 3 5 4 4 2 4;}\r\n/* Style Definitions */\r\np.MsoNormal, li.MsoNormal, div.MsoNormal\r\n\t{margin:0cm;\r\n\tmargin-bottom:.0001pt;\r\n\tfont-size:11.0pt;\r\n\tfont-family:\"Calibri\",sans-serif;\r\n\tmso-fareast-language:EN-US;}\r\nh3\r\n\t{mso-style-priority:9;\r\n\tmso-style-link:\"Heading 3 Char\";\r\n\tmso-margin-top-alt:auto;\r\n\tmargin-right:0cm;\r\n\tmso-margin-bottom-alt:auto;\r\n\tmargin-left:0cm;\r\n\tfont-size:13.5pt;\r\n\tfont-family:\"Times New Roman\",serif;}\r\na:link, span.MsoHyperlink\r\n\t{mso-style-priority:99;\r\n\tcolor:#0563C1;\r\n\ttext-decoration:underline;}\r\na:visited,span.MsoHyperlinkFollowed\r\n\t{mso-style-priority:99;\r\n\tcolor:#954F72;\r\n\ttext-decoration:underline;}\r\nspan.Heading3Char\r\n\t{mso-style-name:\"Heading 3 Char\";\r\n\tmso-style-priority:9;\r\n\tmso-style-link:\"Heading 3\";\r\n\tfont-family:\"Times New Roman\",serif;\r\n\tmso-fareast-language:FR;\r\n\tfont-weight:bold;}\r\nspan.EmailStyle18\r\n\t{mso-style-type:personal;\r\n\tfont-family:\"Calibri\",sans-serif;\r\n\tcolor:windowtext;}\r\nspan.EmailStyle19\r\n\t{mso-style-type:personal-reply;\r\n\tfont-family:\"Calibri\",sans-serif;\r\n\tcolor:#1F497D;}\r\n.MsoChpDefault\r\n\t{mso-style-type:export-only;\r\n\tfont-size:10.0pt;}\r\n@page WordSection1\r\n\t{size:612.0pt 792.0pt;\r\n\tmargin:70.85pt 70.85pt 70.85pt 70.85pt;}\r\ndiv.WordSection1\r\n\t{page:WordSection1;}\r\n--></style><!--[if gte mso 9]><xml>\r\n<o:shapedefaults v:ext=\"edit\" spidmax=\"1026\" />\r\n</xml><![endif]--><!--[if gte mso 9]><xml>\r\n<o:shapelayout v:ext=\"edit\">\r\n<o:idmap v:ext=\"edit\" data=\"1\" />\r\n</o:shapelayout></xml><![endif]--></head><body lang=FR link=\"#0563C1\" vlink=\"#954F72\">Here is the body content which seems fine so far</body></html>";
