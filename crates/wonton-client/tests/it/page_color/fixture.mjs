import vm from 'node:vm';
import readline from 'node:readline';
import assert from 'node:assert/strict';

// 没有浏览器：页面只剩“视口顶上一点叠着哪些元素、各自的计算样式”，脚本读到的就是这些。
const element = (backgroundColor, opacity = '1', parentElement = null) =>
  ({ style: { backgroundColor, opacity }, parentElement });

function page(script) {
  const messages = [];
  const timers = [];
  const html = element('rgba(0, 0, 0, 0)');
  const body = element('rgba(0, 0, 0, 0)', '1', html);
  const state = { stack: () => [body, html], themes: [], html, body, messages };
  const document = {
    documentElement: html, body,
    elementsFromPoint: x => state.stack(x),
    querySelectorAll: () => state.themes.map(content => ({ getAttribute: name => (name === 'content' ? content : null) })),
    createElement: () => ({ getContext: () => null }),
    addEventListener() {},
  };
  const context = vm.createContext({
    document, innerWidth: 400,
    getComputedStyle: target => target.style,
    matchMedia: () => ({ matches: false, addEventListener() {} }),
    MutationObserver: class { constructor(changed) { state.changed = changed; } observe() {} },
    CSS: { supports: () => true },
    setTimeout: run => timers.push(run),
    addEventListener() {},
    wontonPageColor: { postMessage: message => messages.push(message) },
  });
  context.window = context;
  context.top = context;
  vm.runInContext(script, context);
  state.settle = () => { state.changed(); while (timers.length) timers.shift()(); return messages.at(-1); };
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

  // 顶上什么都没涂：用声明的 theme-color（半透明的不算）；连它也没有就报空串。
  view = page(script);
  view.themes = ['rgba(0, 0, 0, 0.5)', 'rgb(42, 111, 151)'];
  assert.equal(view.settle(), '#2a6f97');
  view.themes = [];
  assert.equal(view.settle(), '');
}
console.log(JSON.stringify({ passed: true }));
