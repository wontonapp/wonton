// 报给壳“页面最顶上实际显示的颜色”，整页网页框上面让出来的状态栏那一条照它涂（ADR-0095）。
//
// 不读 theme-color 优先：页面声明的不一定是它顶栏的颜色（Navidrome 0.58 报主题色的浅色档，顶栏是深色档）。
// 读法是在视口最顶上一行取点，把叠在那里的元素的背景色从上往下合成（source-over），直到不透明：
// - 每层的不透明度是 background-color 的 alpha 乘上它和所有祖先的 opacity（opacity 作用在整棵子树上）。
// - background-image（渐变、图片）读不出颜色，只算那个元素的 background-color；图片、视频、canvas 元素同理当透明。
// - 叠完还透着：html、body 没被点到时补上它们的背景色（body 有外边距时点不到它，但它的背景会铺满画布），
//   再不行用页面声明的 theme-color（带 media 的按当前深浅色挑，半透明和写错的不算），都没有就报空串。
// - 取三点（1/4、1/2、3/4 宽）：左右两点一致就用它，否则以中间为准。状态栏的时间和电量在两侧，顶栏正中
//   偶尔有贴顶的标签或徽标，两侧一致时两侧说了算；两侧不一致（一侧有贴顶的按钮）时中间最可能是顶栏本身。
// - pointer-events: none 的元素 elementsFromPoint 不返回，这样的遮罩算不进来。
//
// 消息是 `#rrggbb` 或空串（没有可用的颜色），颜色没变不重发。只在主框架跑。
// 重算用 setTimeout 合并，不用 requestAnimationFrame：加载中的 WebView 是隐藏的，那时候它不触发。
(() => {
  const channel = window.wontonPageColor;
  if (window !== window.top || !channel) return;
  const SOLID = 0.995;
  const WAIT_MS = 100;
  let context;
  // 计算样式里常见的是 rgb()/rgba()，直接拆，alpha 不丢精度；别的写法（十六进制、颜色名、oklch 等）画成一个像素再读回来。
  const drawn = value => {
    if (context === undefined) {
      const canvas = document.createElement('canvas');
      canvas.width = canvas.height = 1;
      context = canvas.getContext('2d', { willReadFrequently: true }) || null;
    }
    if (!context || !CSS.supports('color', value)) return null;
    context.clearRect(0, 0, 1, 1);
    context.fillStyle = value;
    context.fillRect(0, 0, 1, 1);
    const [red, green, blue, alpha] = context.getImageData(0, 0, 1, 1).data;
    return [red, green, blue, alpha / 255];
  };
  const plain = /^rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*(?:,\s*([\d.]+)\s*)?\)$/;
  const parse = value => {
    if (!value) return null;
    const match = plain.exec(value);
    if (!match) return drawn(value);
    return [Number(match[1]), Number(match[2]), Number(match[3]), match[4] === undefined ? 1 : Number(match[4])];
  };
  const declared = () => {
    for (const meta of document.querySelectorAll('meta[name="theme-color" i]')) {
      const media = meta.getAttribute('media');
      if (media && !matchMedia(media).matches) continue;
      const color = parse(meta.getAttribute('content'));
      if (color && color[3] >= SOLID) return color;
    }
    return null;
  };
  const sample = (x, opacities) => {
    let red = 0, green = 0, blue = 0, alpha = 0;
    const under = (color, strength) => {
      const share = (1 - alpha) * Math.min(Math.max(strength, 0), 1);
      red += share * color[0];
      green += share * color[1];
      blue += share * color[2];
      alpha += share;
    };
    const opacity = element => {
      if (!element) return 1;
      let value = opacities.get(element);
      if (value === undefined) {
        const own = parseFloat(getComputedStyle(element).opacity);
        value = (own < 1 ? own : 1) * opacity(element.parentElement);
        opacities.set(element, value);
      }
      return value;
    };
    const layer = element => {
      const color = parse(getComputedStyle(element).backgroundColor);
      if (color && color[3] > 0) under(color, color[3] * opacity(element));
    };
    const hit = document.elementsFromPoint(x, 1);
    for (const element of hit) {
      if (alpha >= SOLID) break;
      layer(element);
    }
    for (const element of [document.documentElement, document.body]) {
      if (alpha < SOLID && element && !hit.includes(element)) layer(element);
    }
    if (alpha < SOLID) {
      const base = declared();
      if (!base) return '';
      under(base, 1);
    }
    return '#' + [red, green, blue]
      .map(part => Math.min(255, Math.round(part / alpha)).toString(16).padStart(2, '0')).join('');
  };
  const shown = () => {
    const width = innerWidth;
    const opacities = new Map();
    const [middle, left, right] = [width / 2, width / 4, width * 3 / 4].map(x => sample(x, opacities));
    return left && left === right ? left : middle;
  };
  let last;
  let queued = false;
  const report = () => {
    queued = false;
    let color = '';
    try { color = shown(); } catch (_) { color = ''; }
    if (color === last) return;
    last = color;
    channel.postMessage(color);
  };
  const schedule = () => {
    if (queued) return;
    queued = true;
    setTimeout(report, WAIT_MS);
  };
  // 盯整份文档：顶上那一条是谁的颜色事先不知道（顶栏、遮罩、换主题时换的 class）。变动再多，WAIT_MS 里也只算一次。
  new MutationObserver(schedule).observe(document, {
    childList: true, subtree: true,
    attributes: true, attributeFilter: ['class', 'style', 'name', 'content', 'media', 'hidden'],
  });
  document.addEventListener('DOMContentLoaded', schedule);
  addEventListener('load', schedule);
  addEventListener('pageshow', schedule);
  addEventListener('resize', schedule);
  // 捕获阶段：页面里自己的滚动容器滚动、样式表加载完都不冒泡。
  document.addEventListener('scroll', schedule, { capture: true, passive: true });
  document.addEventListener('load', schedule, true);
  // 颜色带过渡时，改 class 后头一次算到的是过渡中途的颜色。
  document.addEventListener('transitionend', schedule, true);
  document.addEventListener('animationend', schedule, true);
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', schedule);
  schedule();
})();
