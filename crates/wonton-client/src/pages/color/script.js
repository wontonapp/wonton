// 报给壳“页面最顶上实际显示的颜色”，整页网页框上面让出来的状态栏那一条照它涂（ADR-0095）。
//
// 不读 theme-color 优先：页面声明的不一定是它顶栏的颜色（Navidrome 0.58 报主题色的浅色档，顶栏是深色档）。
// 读法是在视口最顶上一行取点，把叠在那里的元素的背景色从上往下合成（source-over），直到不透明：
// - 每层的不透明度是 background-color 的 alpha 乘上它和所有祖先的 opacity（opacity 作用在整棵子树上）。
// - background-image（渐变、图片）读不出颜色，只算那个元素的 background-color；图片、视频、canvas 元素同理当透明。
// - 叠完还透着：html、body 没被点到时补上它们的背景色（body 有外边距时点不到它，但它的背景会铺满画布），
//   再不行用页面声明的 theme-color（带 media 的按当前深浅色挑，半透明和写错的不算），都没有就报空串。
//   声明的颜色要等页面加载完再过 SETTLE_MS 才用：单页应用画出顶栏之前顶上什么都没涂，这时报它，
//   顶条会先闪一下声明的颜色（Navidrome 是浅色）再换成顶栏的；那段时间屏幕上实际是壳的底色，报空串。
// - 取三点（1/4、1/2、3/4 宽）：左右两点一致就用它，否则以中间为准。状态栏的时间和电量在两侧，顶栏正中
//   偶尔有贴顶的标签或徽标，两侧一致时两侧说了算；两侧不一致（一侧有贴顶的按钮）时中间最可能是顶栏本身。
// - pointer-events: none 的元素 elementsFromPoint 不返回，这样的遮罩算不进来。
// - 顶栏在 Shadow DOM 里时（Home Assistant 等 Web Components 应用），document.elementsFromPoint 只给到宿主元素：
//   逐层进开放的 shadow root 再取，里面的元素排在宿主前面；进过的 shadow root 一并盯变动。封闭的进不去，只算到宿主。
//
// 消息是 `#rrggbb` 或空串（没有可用的颜色），颜色没变不重发。只在主框架跑。
// 重算用 setTimeout 合并，不用 requestAnimationFrame：加载中的 WebView 是隐藏的，那时候它不触发。
// 每次重算要取点、读计算样式，会让布局提前做掉，所以：
// - 页面加载完以后还隐藏着（栈里压在下面的页、关掉后留着的页）就不算，等它显示出来马上算一次。
// - DOM 一直在变（播放进度条、脚本驱动的动画）而颜色没变时，两次重算的间隔从 WAIT_MS 翻倍放宽到 SLOW_MS；
//   颜色变了或者变动停了一阵就回到 WAIT_MS。
(() => {
  const channel = window.wontonPageColor;
  if (window !== window.top || !channel) return;
  const SOLID = 0.995;
  const WAIT_MS = 100;
  const SLOW_MS = 800;
  const SETTLE_MS = 300;
  const SETTLE_WITHOUT_LOAD_MS = 3000;
  const WATCHED = {
    childList: true, subtree: true,
    attributes: true, attributeFilter: ['class', 'style', 'name', 'content', 'media', 'hidden'],
  };
  let settled = false;
  let observer;
  const roots = new WeakSet();
  const watch = root => {
    if (roots.has(root)) return;
    roots.add(root);
    observer.observe(root, WATCHED);
  };
  // 按画出来的树往上找：放进 slot 的元素跟着 slot，shadow root 里最外层的元素跟着宿主。
  const above = element =>
    element.assignedSlot || element.parentElement || (element.parentNode && element.parentNode.host) || null;
  // root.elementsFromPoint 也会带出 root 外面的元素（宿主和它的祖先），那些由外层自己列。
  const stack = (root, x) => {
    const found = [];
    for (const element of root.elementsFromPoint(x, 1)) {
      if (found.includes(element) || (root !== document && element.getRootNode() !== root)) continue;
      const inner = element.shadowRoot;
      if (inner && typeof inner.elementsFromPoint === 'function') {
        watch(inner);
        found.push(...stack(inner, x));
      }
      found.push(element);
    }
    return found;
  };
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
        value = (own < 1 ? own : 1) * opacity(above(element));
        opacities.set(element, value);
      }
      return value;
    };
    const layer = element => {
      const color = parse(getComputedStyle(element).backgroundColor);
      if (color && color[3] > 0) under(color, color[3] * opacity(element));
    };
    const hit = stack(document, x);
    for (const element of hit) {
      if (alpha >= SOLID) break;
      layer(element);
    }
    for (const element of [document.documentElement, document.body]) {
      if (alpha < SOLID && element && !hit.includes(element)) layer(element);
    }
    if (alpha < SOLID) {
      const base = settled && declared();
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
  let timer;
  let due = 0;
  let ran = 0;
  let delay = WAIT_MS;
  const report = () => {
    timer = undefined;
    if (document.hidden && document.readyState === 'complete') return;
    ran = Date.now();
    let color = '';
    try { color = shown(); } catch (_) { color = ''; }
    if (color === last) {
      delay = Math.min(delay * 2, SLOW_MS);
      return;
    }
    delay = WAIT_MS;
    last = color;
    channel.postMessage(color);
  };
  const schedule = wait => {
    const at = Date.now() + wait;
    if (timer !== undefined) {
      if (at >= due) return;
      clearTimeout(timer);
    }
    due = at;
    timer = setTimeout(report, wait);
  };
  const soon = () => schedule(WAIT_MS);
  // 盯整份文档：顶上那一条是谁的颜色事先不知道（顶栏、遮罩、换主题时换的 class）。
  observer = new MutationObserver(() => {
    if (Date.now() - ran > 2 * SLOW_MS) delay = WAIT_MS;
    schedule(delay);
  });
  watch(document);
  const settle = wait => () => setTimeout(() => {
    if (settled) return;
    settled = true;
    soon();
  }, wait);
  document.addEventListener('DOMContentLoaded', soon);
  document.addEventListener('DOMContentLoaded', settle(SETTLE_WITHOUT_LOAD_MS));
  addEventListener('load', soon);
  addEventListener('load', settle(SETTLE_MS));
  // 从前进后退缓存里回来的页，脚本的状态原样留着，壳那边却已经换成了后一页报的颜色：忘掉上次报的，重报一遍。
  addEventListener('pageshow', event => {
    if (event.persisted) last = undefined;
    soon();
  });
  addEventListener('resize', soon);
  document.addEventListener('visibilitychange', () => { if (!document.hidden) schedule(0); });
  // 捕获阶段：页面里自己的滚动容器滚动、样式表加载完都不冒泡。
  document.addEventListener('scroll', soon, { capture: true, passive: true });
  document.addEventListener('load', soon, true);
  // 颜色带过渡时，改 class 后头一次算到的是过渡中途的颜色。
  document.addEventListener('transitionend', soon, true);
  document.addEventListener('animationend', soon, true);
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', soon);
  soon();
})();
