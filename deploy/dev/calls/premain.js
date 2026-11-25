// Element Call 会检测 RTCPeerConnection 构造器被替换并整页重置（05:1x 实测），
// 改挂原型方法 getStats：EC 自己每秒轮询 getStats，this 就是活连接，无构造器改动。
// join_call 用：底部 Join call 按钮的视口中心坐标（CDP Input.dispatchMouseEvent 用 CSS 像素）。
'use strict';
(() => {
  window.__pcs = [];
  const proto = window.RTCPeerConnection && window.RTCPeerConnection.prototype;
  if (proto && !proto.__wontonHooked) {
    const orig = proto.getStats;
    proto.getStats = function (...args) {
      if (window.__pcs.indexOf(this) === -1) window.__pcs.push(this);
      return orig.apply(this, args);
    };
    proto.__wontonHooked = true;
  }
})();

window.__joinRect = () => {
  const els = [...document.querySelectorAll('button,[role=button],div,span')];
  const b = els.find(e => /^join call$/i.test((e.textContent || '').trim()));
  if (!b) return null;
  const r = b.getBoundingClientRect();
  return { x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2) };
};

// 首次打开盖的 “Got it” 介绍层：取最小同名元素的中心，点掉它。
window.__gotItRect = () => {
  const c = [...document.querySelectorAll('button,[role=button],div,span')]
    .filter(e => /^got it$/i.test((e.textContent || '').trim()));
  if (!c.length) return null;
  const e = c.sort((a, b) => {
    const ra = a.getBoundingClientRect(), rb = b.getBoundingClientRect();
    return ra.width * ra.height - rb.width * rb.height;
  })[0];
  const r = e.getBoundingClientRect();
  return { x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2) };
};
