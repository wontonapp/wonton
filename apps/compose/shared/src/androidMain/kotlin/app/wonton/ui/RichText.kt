package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withLink
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.em
import app.wonton.protocol.RichBlock
import app.wonton.protocol.RichBlockKind
import app.wonton.protocol.RichSpan
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

private const val CODE_SCALE = 0.9f

/**
 * 排好版的消息文字。块和片段由核心给出，这里只管画。链接和代码底色都跟着气泡里文字的颜色。
 * `trailingReserve` 是末尾接的一段透明文字，给压在最后一行上的时间留位置。
 */
@Composable
fun RichText(blocks: List<RichBlock>, trailingReserve: String? = null) {
    Column {
        blocks.forEachIndexed { index, block ->
            if (index > 0) Spacer(Modifier.height(gap(blocks[index - 1], block)))
            RichBlockView(block, if (index == blocks.lastIndex) trailingReserve else null)
        }
    }
}

@Composable
private fun gap(previous: RichBlock, next: RichBlock): Dp =
    if (previous.kind is RichBlockKind.ListItem && next.kind is RichBlockKind.ListItem) {
        WontonTheme.spacing.s4
    } else {
        WontonTheme.spacing.s8
    }

@Composable
private fun RichBlockView(block: RichBlock, reserve: String?) {
    val ink = LocalContentColor.current
    val spacing = WontonTheme.spacing
    val bar = WontonTheme.size.quoteBar
    val quote = if (block.quoted) {
        Modifier
            .drawBehind {
                drawRect(
                    ink.copy(alpha = WontonTheme.ratio.secondaryContent),
                    size = Size(bar.toPx(), size.height),
                )
            }
            .padding(start = spacing.s12)
    } else {
        Modifier
    }
    Box(quote.padding(start = spacing.s16 * block.depth.toInt())) {
        val reserveStyle = SpanStyle(color = Color.Transparent, fontSize = WontonTheme.typography.caption.fontSize)
        val inline = inlineText(block.spans, ink.copy(alpha = WontonTheme.ratio.codeFill), reserve, reserveStyle)
        when (val kind = block.kind) {
            RichBlockKind.Paragraph -> Text(inline)
            RichBlockKind.Heading -> Text(inline, style = WontonTheme.typography.itemTitle)
            is RichBlockKind.ListItem -> Row(horizontalArrangement = Arrangement.spacedBy(spacing.s8)) {
                Text(kind.marker, Modifier.alignByBaseline())
                Text(inline, Modifier.alignByBaseline())
            }
            RichBlockKind.Code -> CodeBlock(block.spans.firstOrNull()?.text.orEmpty(), ink)
            is RichBlockKind.Table -> RichTable(kind.rows)
            RichBlockKind.Rule -> Box(
                Modifier.fillMaxWidth().height(WontonTheme.size.hairline)
                    .background(WontonTheme.colors.separator.toColor()),
            )
        }
    }
}

/** 列宽取这一列最宽的格子，行高取这一行最高的格子：中文和英文的行高不一样，分开摆会错行。 */
@Composable
private fun RichTable(rows: List<List<String>>) {
    val spacing = WontonTheme.spacing
    val hairline = WontonTheme.size.hairline
    val columns = rows.firstOrNull()?.size ?: return
    val separator = WontonTheme.colors.separator.toColor()
    Layout(
        content = {
            rows.forEachIndexed { index, row ->
                row.forEach { cell ->
                    Text(cell, fontWeight = if (index == 0) FontWeight.SemiBold else null, softWrap = false)
                }
            }
            Box(Modifier.background(separator))
        },
        modifier = Modifier.horizontalScroll(rememberScrollState()),
    ) { measurables, _ ->
        val cells = measurables.dropLast(1).map { it.measure(Constraints()) }.chunked(columns)
        val widths = List(columns) { column -> cells.maxOf { it[column].width } }
        val heights = cells.map { row -> row.maxOf { it.height } }
        val columnGap = spacing.s16.roundToPx()
        val rowGap = spacing.s4.roundToPx()
        val line = hairline.roundToPx()
        val width = widths.sum() + columnGap * (columns - 1)
        val rule = measurables.last().measure(Constraints.fixed(width, line))
        layout(width, heights.sum() + rowGap * heights.size + line) {
            var y = 0
            cells.forEachIndexed { index, row ->
                var x = 0
                row.forEachIndexed { column, cell ->
                    cell.placeRelative(x, y + (heights[index] - cell.height) / 2)
                    x += widths[column] + columnGap
                }
                y += heights[index] + rowGap
                if (index == 0) {
                    rule.placeRelative(0, y)
                    y += line + rowGap
                }
            }
        }
    }
}

/** 不折行，放不下时横向滚动。 */
@Composable
private fun CodeBlock(text: String, ink: Color) {
    val spacing = WontonTheme.spacing
    Box(
        Modifier
            .background(ink.copy(alpha = WontonTheme.ratio.codeFill), RoundedCornerShape(WontonTheme.radii.control))
            .horizontalScroll(rememberScrollState())
            .padding(horizontal = spacing.s12, vertical = spacing.s8),
    ) {
        Text(text, style = WontonTheme.typography.supporting, fontFamily = FontFamily.Monospace, softWrap = false)
    }
}

private fun inlineText(
    spans: List<RichSpan>,
    codeFill: Color,
    reserve: String?,
    reserveStyle: SpanStyle,
): AnnotatedString = buildAnnotatedString {
    spans.forEach { span ->
        val style = SpanStyle(
            fontWeight = if (span.bold) FontWeight.Bold else null,
            fontStyle = if (span.italic) FontStyle.Italic else null,
            fontFamily = if (span.code) FontFamily.Monospace else null,
            // 等宽字体看起来比正文大。
            fontSize = if (span.code) CODE_SCALE.em else TextUnit.Unspecified,
            background = if (span.code) codeFill else Color.Unspecified,
            textDecoration = if (span.strike) TextDecoration.LineThrough else null,
        )
        val link = span.link
        if (link == null) {
            withStyle(style) { append(span.text) }
        } else {
            val underline = TextLinkStyles(SpanStyle(textDecoration = TextDecoration.Underline))
            withLink(LinkAnnotation.Url(link.url, underline)) { withStyle(style) { append(span.text) } }
        }
    }
    if (reserve != null) withStyle(reserveStyle) { append(reserve) }
}
