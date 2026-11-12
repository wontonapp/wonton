var validLangs = ["ar","bg","ca","ca@valencia","cs","da","de","el","en","en-GB","eo","es","eu","fa","fil","fr","fy","ga","gl","he-IL","hi","hr","hu","id","it","ja","ko-KR","lt","nl","pl","pt-BR","pt-PT","ro-RO","ru","sk","sl","sv","tr","uk","zh-CN","zh-HK","zh-TW"]
;

// 云吞：上行是上游 v2.1.7 的 gui/default/assets/lang/valid-langs.js（升级镜像时重新取来，
// 这行以上的内容保持原样）；下面在页面顶部加一块局域网地址提示，地址来自 /wonton-lan.json
// （configs 按 WONTON_LAN_ADDRESSES 生成），取不到时把原因显示出来。
(function () {
  var panel = document.createElement('div');
  panel.className = 'container content';
  panel.innerHTML =
    '<div class="row"><div class="col-md-12"><div class="panel panel-info">' +
    '<div class="panel-heading"><h3 class="panel-title">' +
    '<span class="fas fa-network-wired"></span>&nbsp;<span translate>Address</span></h3></div>' +
    '<div class="panel-body"><p><code id="wonton-lan-address"><span translate>Loading...</span></code></p></div>' +
    '</div></div></div>';
  var anchor = document.querySelector('.container.content');
  if (anchor) {
    anchor.before(panel);
  } else {
    // 上游改了页面结构也要显示出来，不能静默消失。
    document.body.insertBefore(panel, document.body.firstChild);
  }

  var out = document.getElementById('wonton-lan-address');

  function show(text) {
    out.textContent = text;
  }

  fetch('/wonton-lan.json')
    .then(function (response) {
      if (!response.ok) {
        throw new Error('HTTP ' + response.status);
      }
      return response.json();
    })
    .then(function (data) {
      var addresses = String(data.addresses || '')
        .split(',')
        .map(function (address) { return address.trim(); })
        .filter(Boolean);
      if (!addresses.length) {
        throw new Error('核心没有给出地址 / no address from the core');
      }
      show(addresses.map(function (address) {
        // IPv6 字面量要加方括号才能拼成 URL。
        return 'tcp://' + (address.indexOf(':') > -1 ? '[' + address + ']' : address) + ':22000';
      }).join('、'));
    })
    .catch(function (error) {
      show('读取失败 / Failed: ' + (error && error.message ? error.message : String(error)));
    });
})();
