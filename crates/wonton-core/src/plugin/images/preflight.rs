//! 交给 usvg 之前检查一遍 SVG 图标，给作者清楚的报错：根元素、`<use>` 的写法和展开后的规模、会按引用次数复制内容的
//! 功能、样式表。这里不追求和 usvg 认得一模一样（只认一种写法，拿不准的一律拒绝），也挡不住所有费时费内存的写法（比如
//! 很细的虚线），硬上限是处理图片的子进程的资源上限（见 [`crate::plugin::image_worker`]）。

use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use resvg::usvg::roxmltree::{Document, Node, NodeId};
use simplecss::{AttributeOperator, Element, PseudoClass, Selector, StyleSheet};

/// SVG 展开 `<use>` 后最多这么多元素。图标用不到这么多；层层引用的 `<use>` 能用几 KB 的文件展开出上亿个
/// 元素，resvg 自己的上限（一百万）要占几百 MB 内存。
const MAX_ELEMENTS: u64 = 20_000;
/// 展开后最多这么多层：usvg 和 resvg 都按层递归，resvg 还给带透明度、剪切、蒙版的每一层分配一张画布。
const MAX_DEPTH: u32 = 64;
/// 每个渐变最多这么多个 `<stop>`，见 [`allowed`]。
const MAX_GRADIENT_STOPS: usize = 64;
/// 样式表最多这么多条规则（`a, b {…}` 算两条）：usvg 拿每条规则去匹配每个元素。
const MAX_STYLE_RULES: usize = 256;
const SVG_NS: &str = "http://www.w3.org/2000/svg";

pub(super) fn check(doc: &Document) -> Result<(), String> {
    let root = doc.root_element();
    if root.tag_name().name() != "svg" || root.tag_name().namespace() != Some(SVG_NS) {
        return Err(format!("根元素要是 <svg xmlns=\"{SVG_NS}\">"));
    }
    style_sheets(doc)?;
    Preflight::new(doc).count(root, 0)?;
    Ok(())
}

/// 样式表只收不带组合符的选择器（`.cls-1`、`rect.a`、`#a`）：simplecss 匹配后代选择器时逐层回溯，几百字节的样式表
/// 配上几十层嵌套就能算上几分钟。按 usvg 的找法找样式表（本地名是 `style` 的元素，`type` 不写或是 `text/css`，取第一段
/// 文字），用同一个 simplecss 解析，检查的就是 usvg 要匹配的那些规则。
fn style_sheets(doc: &Document) -> Result<(), String> {
    let mut sheet = StyleSheet::new();
    for node in doc.descendants().filter(|n| n.has_tag_name("style")) {
        if matches!(node.attribute("type"), None | Some("text/css"))
            && let Some(text) = node.text()
        {
            sheet.parse_more(text);
        }
    }
    if sheet.rules.len() > MAX_STYLE_RULES {
        return Err(format!(
            "样式表最多 {MAX_STYLE_RULES} 条规则，这里有 {} 条",
            sheet.rules.len()
        ));
    }
    match sheet.rules.iter().find(|r| has_combinator(&r.selector)) {
        Some(rule) => Err(format!(
            "样式表的选择器 “{}” 带了组合符：只能用 .cls-1 这样不带组合符的选择器",
            rule.selector
        )),
        None => Ok(()),
    }
}

/// 拿一个什么都匹配的元素去试选择器，要往父元素或前一个兄弟元素上找的就是带了组合符。simplecss 不公开选择器的
/// 结构，这样问的就是 usvg 匹配时跑的那段代码。
fn has_combinator(selector: &Selector) -> bool {
    let walked = Cell::new(false);
    selector.matches(&Probe(&walked));
    walked.get()
}

struct Probe<'a>(&'a Cell<bool>);

impl Element for Probe<'_> {
    fn parent_element(&self) -> Option<Self> {
        self.0.set(true);
        None
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        self.0.set(true);
        None
    }

    fn has_local_name(&self, _: &str) -> bool {
        true
    }

    fn attribute_matches(&self, _: &str, _: AttributeOperator<'_>) -> bool {
        true
    }

    fn pseudo_class_matches(&self, _: PseudoClass<'_>) -> bool {
        true
    }
}

/// 展开 `<use>` 之后的规模：元素数，和往下最深的层数。
#[derive(Clone, Copy)]
struct Expanded {
    elements: u64,
    height: u32,
}

/// 数展开 `<use>` 后的元素和层数，拒绝会按引用次数复制内容的功能（见 [`allowed`]）。
struct Preflight<'a, 'input> {
    /// 本地名是 `id` 的属性都登记，不管命名空间（usvg 就是这样找的）；同一个 id 有几个元素时都算，取最大的。
    ids: HashMap<&'a str, Vec<Node<'a, 'input>>>,
    counted: HashMap<NodeId, Expanded>,
    visiting: HashSet<NodeId>,
}

impl<'a, 'input> Preflight<'a, 'input> {
    fn new(doc: &'a Document<'input>) -> Self {
        let mut ids: HashMap<&str, Vec<_>> = HashMap::new();
        for node in doc.descendants().filter(Node::is_element) {
            for attr in node.attributes().filter(|a| a.name() == "id") {
                ids.entry(attr.value()).or_default().push(node);
            }
        }
        Self {
            ids,
            counted: HashMap::new(),
            visiting: HashSet::new(),
        }
    }

    fn count(&mut self, node: Node<'a, 'input>, depth: u32) -> Result<Expanded, String> {
        let expanded = match self.counted.get(&node.id()) {
            Some(&expanded) => expanded,
            None => self.expand(node, depth)?,
        };
        if depth + expanded.height > MAX_DEPTH {
            return Err(format!("SVG 嵌套超过 {MAX_DEPTH} 层"));
        }
        Ok(expanded)
    }

    fn expand(&mut self, node: Node<'a, 'input>, depth: u32) -> Result<Expanded, String> {
        if depth > MAX_DEPTH {
            return Err(format!("SVG 嵌套超过 {MAX_DEPTH} 层"));
        }
        if !self.visiting.insert(node.id()) {
            return Err("SVG 里的 <use> 循环引用".to_owned());
        }
        allowed(node)?;
        let mut expanded = Expanded {
            elements: 1,
            height: 1,
        };
        let mut add = |part: Expanded| {
            expanded.elements = expanded.elements.saturating_add(part.elements);
            expanded.height = expanded.height.max(part.height + 1);
            if expanded.elements > MAX_ELEMENTS {
                return Err(format!("SVG 展开 <use> 后超过 {MAX_ELEMENTS} 个元素"));
            }
            Ok(())
        };
        for child in node.children().filter(Node::is_element) {
            add(self.count(child, depth + 1)?)?;
        }
        if is_svg(node, "use") {
            let targets = self.ids.get(use_target(node)?).cloned().unwrap_or_default();
            let mut largest = Expanded {
                elements: 0,
                height: 0,
            };
            for target in targets {
                let part = self.count(target, depth + 1)?;
                largest.elements = largest.elements.max(part.elements);
                largest.height = largest.height.max(part.height);
            }
            if largest.elements > 0 {
                add(largest)?;
            }
        }
        self.visiting.remove(&node.id());
        self.counted.insert(node.id(), expanded);
        Ok(expanded)
    }
}

/// usvg 把没有命名空间的元素也当 SVG 元素。
fn is_svg(node: Node, name: &str) -> bool {
    node.tag_name().name() == name && matches!(node.tag_name().namespace(), None | Some(SVG_NS))
}

/// `<use>` 引用的 id。只认一种写法：只有一个 href（`href` 或 `xlink:href`，别的命名空间里的也算），值是 `#` 加不含空白的
/// id。几个 href 时 usvg 和这里可能各取一个；空白是 usvg 和这里断开 id 的地方可能不同。
fn use_target<'a>(node: Node<'a, '_>) -> Result<&'a str, String> {
    let mut hrefs = node.attributes().filter(|a| a.name() == "href");
    let (Some(href), None) = (hrefs.next(), hrefs.next()) else {
        return Err("<use> 要有且只有一个 href（href 或 xlink:href）".to_owned());
    };
    match href.value().strip_prefix('#') {
        Some(id) if !id.is_empty() && !id.contains(char::is_whitespace) => Ok(id),
        _ => Err(format!(
            "<use> 的 href 只能写成 #id，id 里不能有空白：{:?}",
            href.value()
        )),
    }
}

/// 拒绝 usvg 会按引用次数复制内容的功能，图标都用不到：usvg 给每个用到它们的元素各转换一份按 objectBoundingBox
/// 换算的剪切路径、蒙版、图案和滤镜，给路径的每个顶点各放一份标记，给每条路径各复制一份按 objectBoundingBox
/// 换算的渐变。不拒绝的话，几 KB 的 SVG 就能让 usvg 用掉几 GB 内存，`<use>` 的计数也拦不住。
fn allowed(node: Node) -> Result<(), String> {
    for name in ["marker", "pattern", "filter"] {
        if is_svg(node, name) {
            return Err(format!("图标不能用 <{name}>"));
        }
    }
    if is_svg(node, "clipPath") && !user_space(node, "clipPathUnits", true) {
        return Err("<clipPath> 的 clipPathUnits 只能是 userSpaceOnUse".to_owned());
    }
    if is_svg(node, "mask")
        && !(user_space(node, "maskUnits", false) && user_space(node, "maskContentUnits", true))
    {
        return Err(
            "<mask> 要写 maskUnits=\"userSpaceOnUse\"，maskContentUnits 只能是 userSpaceOnUse"
                .to_owned(),
        );
    }
    if is_svg(node, "linearGradient") || is_svg(node, "radialGradient") {
        let stops = node.children().filter(|c| is_svg(*c, "stop")).count();
        if stops > MAX_GRADIENT_STOPS {
            return Err(format!(
                "渐变最多 {MAX_GRADIENT_STOPS} 个 <stop>，这个有 {stops} 个"
            ));
        }
    }
    Ok(())
}

/// 这个名字的属性写了 `userSpaceOnUse`；`absent_ok` 时也可以不写（默认就是）。只认原样的 `userSpaceOnUse`：usvg
/// 读不懂的值按默认值处理，而 `maskUnits` 的默认值是 objectBoundingBox。
///
/// 算“写了”只看没有命名空间和 SVG 命名空间的：usvg 不读别的命名空间里的属性，只写在那里等于没写（xlink 和 xml 命名
/// 空间里的它也读，这里不认这种写法）。值要求所有命名空间里同名的都是 `userSpaceOnUse`，不管 usvg 读到的是哪一个。
fn user_space(node: Node, name: &str, absent_ok: bool) -> bool {
    let mut written = false;
    for attr in node.attributes().filter(|a| a.name() == name) {
        if attr.value() != "userSpaceOnUse" {
            return false;
        }
        written |= matches!(attr.namespace(), None | Some(SVG_NS));
    }
    written || absent_ok
}
