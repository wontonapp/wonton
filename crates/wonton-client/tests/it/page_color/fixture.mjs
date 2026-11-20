import vm from 'node:vm';
import readline from 'node:readline';
import assert from 'node:assert/strict';

// 没有浏览器：页面只剩“视口顶上一点叠着哪些元素、各自的计算样式”，脚本读到的就是这些。
const element = (backgroundColor, opacity = '1', parentElement = null) =>
  ({ style: { backgroundColor, opacity }, parentElement, getRootNode: () => null });

function page(script) {
  const messages = [];
  const timers = new Map();
  const listeners = {};
  let timerId = 0;
  const html = element('rgba(0, 0, 0, 0)');
  const body = element('rgba(0, 0, 0, 0)', '1', html);
  const state = { stack: () => [body, html], themes: [], html, body, messages, watched: [] };
  const document = {
    documentElement: html, body,
    elementsFromPoint: x => state.stack(x),
    querySelectorAll: () => state.themes.map(content => ({ getAttribute: name => (name === 'content' ? content : null) })),
    createElement: () => ({ getContext: () => null }),
    addEventListener: (name, run) => (listeners[name] ||= []).push(run),
  };
  const context = vm.createContext({
    document, innerWidth: 400,
    getComputedStyle: target => target.style,
    matchMedia: () => ({ matches: false, addEventListener() {} }),
    MutationObserver: class {
      constructor(changed) { state.changed = changed; }
      observe(root) { state.watched.push(root); }
    },
    CSS: { supports: () => true },
    setTimeout: run => { timers.set(++timerId, run); return timerId; },
    clearTimeout: id => timers.delete(id),
    addEventListener: (name, run) => (listeners[name] ||= []).push(run),
    wontonPageColor: { postMessage: message => messages.push(message) },
  });
  context.window = context;
  context.top = context;
  vm.runInContext(script, context);
  const drain = () => {
    while (timers.size) {
      const [id, run] = timers.entries().next().value;
      timers.delete(id);
      run();
    }
  };
  state.fire = (name, event = {}) => { for (const run of listeners[name] || []) run(event); drain(); return messages.at(-1); };
  state.settle = () => { state.changed(); drain(); return messages.at(-1); };
  state.document = document;
  return state;
}

const lines = readline.createInterface({ input: process.stdin });
for await (const line of lines) {
  const { script } = JSON.parse(line);

  // 页面声明的 theme-color 是浅色，固定在顶上的栏是深色：报栏的颜色。
  let view = page(script);
  const bar = element('rgb(31, 58, 95)', '1', view.body);
  view.themes = ['rgb(144, 202, 249)'];
  view.body.style.backgroundColor = 'rgb(255, 255, 255)';
  view.stack = () => [bar, view.body, view.html];
  assert.equal(view.settle(), '#1f3a5f');

  // 半透明的遮罩盖在栏上：报合成出来的颜色。
  const veil = element('rgba(255, 255, 255, 0.5)', '1', view.body);
  view.stack = () => [veil, bar, view.body, view.html];
  assert.equal(view.settle(), '#8f9daf');

  // 遮罩放在半透明的容器里：容器的 opacity 也算。
  const holder = element('rgba(0, 0, 0, 0)', '0.5', view.body);
  const solid = element('rgb(255, 255, 255)', '1', holder);
  view.stack = () => [solid, holder, bar, view.body, view.html];
  assert.equal(view.settle(), '#8f9daf');

  // 颜色没变不重发。
  const sent = view.messages.length;
  view.settle();
  assert.equal(view.messages.length, sent);

  // 一侧有贴顶的按钮时以中间为准；两侧一致、只有正中不同时两侧说了算。
  const button = element('rgb(200, 0, 0)', '1', bar);
  view.stack = x => (x < 150 ? [button, bar, view.body, view.html] : [bar, view.body, view.html]);
  assert.equal(view.settle(), '#1f3a5f');
  view.stack = x => (x === 200 ? [button, bar, view.body, view.html] : [bar, view.body, view.html]);
  assert.equal(view.settle(), '#1f3a5f');

  // body 有外边距，顶上点不到它：它的背景照样铺在最底下，排在 theme-color 前面。
  view = page(script);
  view.themes = ['rgb(144, 202, 249)'];
  view.body.style.backgroundColor = 'rgb(23, 50, 77)';
  view.stack = () => [view.html];
  assert.equal(view.settle(), '#17324d');

  // 顶上什么都没涂：页面加载完之前不用声明的 theme-color（单页应用的顶栏这时还没画出来），报空串；
  // 加载完以后用它（半透明的不算）；连它也没有就报空串。
  view = page(script);
  view.themes = ['rgba(0, 0, 0, 0.5)', 'rgb(42, 111, 151)'];
  assert.equal(view.settle(), '');
  assert.equal(view.fire('load'), '#2a6f97');
  view.themes = [];
  assert.equal(view.settle(), '');

  // 顶栏在 shadow root 里：进去取，里面的变动也盯着；宿主半透明时里面的一起算。
  view = page(script);
  const host = element('rgba(0, 0, 0, 0)', '0.5', view.body);
  const shadow = { host, elementsFromPoint: () => [inside, host, view.body, view.html] };
  const inside = { ...element('rgb(255, 255, 255)'), parentNode: shadow, getRootNode: () => shadow };
  host.shadowRoot = shadow;
  view.body.style.backgroundColor = 'rgb(31, 58, 95)';
  view.stack = () => [host, view.body, view.html];
  assert.equal(view.settle(), '#8f9daf');
  assert.ok(view.watched.includes(shadow));

  // 从前进后退缓存里回来：颜色没变也重报一遍，普通的 pageshow 不重报。
  const before = view.messages.length;
  view.fire('pageshow', { persisted: false });
  assert.equal(view.messages.length, before);
  assert.equal(view.fire('pageshow', { persisted: true }), '#8f9daf');
  assert.equal(view.messages.length, before + 1);

  // 加载完以后还隐藏着的页不算，显示出来马上补报。
  view.document.hidden = true;
  view.document.readyState = 'complete';
  view.body.style.backgroundColor = 'rgb(23, 50, 77)';
  view.stack = () => [view.body, view.html];
  view.settle();
  assert.equal(view.messages.length, before + 1);
  view.document.hidden = false;
  assert.equal(view.fire('visibilitychange'), '#17324d');
}
console.log(JSON.stringify({ passed: true }));
