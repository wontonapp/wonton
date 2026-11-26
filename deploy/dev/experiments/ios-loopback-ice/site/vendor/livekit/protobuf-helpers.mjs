// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 1-27 行按职责切出，行为不变。
function _mergeNamespaces(n, m){m.forEach(function(e){e&&typeof e!=='string'&&!Array.isArray(e)&&Object.keys(e).forEach(function(k){if(k!=='default'&&!(k in n)){var d=Object.getOwnPropertyDescriptor(e,k);Object.defineProperty(n,k,d.get?d:{enumerable:true,get:function(){return e[k]}});}})});return Object.freeze(n);}var e = Object.defineProperty;
var h = (i, s, t) => s in i ? e(i, s, {
  enumerable: true,
  configurable: true,
  writable: true,
  value: t
}) : i[s] = t;
var o = (i, s, t) => h(i, typeof s != "symbol" ? s + "" : s, t);
class _ {
  constructor() {
    o(this, "_locking");
    o(this, "_locks");
    this._locking = Promise.resolve(), this._locks = 0;
  }
  isLocked() {
    return this._locks > 0;
  }
  lock() {
    this._locks += 1;
    let s;
    const t = new Promise(l => s = () => {
        this._locks -= 1, l();
      }),
      c = this._locking.then(() => s);
    return this._locking = this._locking.then(() => t), c;
  }
}
export { _, _mergeNamespaces, e, h, o };
