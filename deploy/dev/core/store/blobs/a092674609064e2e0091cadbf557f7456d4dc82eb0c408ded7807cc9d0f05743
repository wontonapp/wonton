#!/usr/bin/perl
# 网页入口（3430）：把请求转给内部端口上的 magpie 本体（3431），在网页 HTML 的 </head> 前插入
# layout.css。固定版本的 magpie 没有自定义样式的设置，页面头只能在这里改。
#
# 只缓冲导航请求拿回的 HTML 200（要插入），其余（API、SSE、图标、XHR）拿到响应头就原样流过去，
# 长连接不断。只有导航请求向上游要未压缩的正文（注入才不用先解 gzip），样式、脚本照旧收 gzip——
# 首次加载 4MB 的正文压到约四分之一，是上游自己量过的。上游连不上回 502。每个连接一个子进程。
use strict;
use warnings;
use IO::Socket::INET;

# 端口和样式路径写死在部署里；环境变量只给本地试验用。
my $listen_port = $ENV{WONTON_WEB_PORT}    // 3430;
my $upstream    = $ENV{WONTON_WEB_UPSTREAM} // '127.0.0.1:3431';
my $css_path    = $ENV{WONTON_LAYOUT_CSS}  // '/wonton/layout.css';

my $css = do {
  open my $fh, '<', $css_path or die "读不到 $css_path：$!\n";
  local $/;
  <$fh>;
};

my $server = IO::Socket::INET->new(
  LocalAddr => '0.0.0.0', LocalPort => $listen_port,
  Listen => 128, ReuseAddr => 1, Proto => 'tcp',
) or die "网页入口端口 $listen_port 监听不了：$!\n";

$SIG{CHLD} = 'IGNORE';

while (1) {
  my $client = $server->accept or next;
  defined(my $pid = fork) or next;
  if ($pid) { close $client; next; }
  close $server;
  serve($client);
  exit 0;
}

sub serve {
  my ($client) = @_;
  $client->autoflush(1);
  my ($method, $path, $headers, $length, $rest, $nav) = read_request($client) or return;

  my ($host, $port) = split /:/, $upstream;
  my $up = IO::Socket::INET->new(PeerAddr => $host, PeerPort => $port, Proto => 'tcp', Timeout => 20);
  unless ($up) {
    my $why = "magpie 连不上\n";
    print $client "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain; charset=utf-8\r\n"
      . "Content-Length: " . length($why) . "\r\nConnection: close\r\n\r\n$why";
    close $client;
    return;
  }
  $up->autoflush(1);

  # 请求体（保存服务商这类 POST）先转发完，再等响应。
  my $request = "$method $path HTTP/1.0\r\nHost: $upstream\r\n" . join('', map { "$_\r\n" } @$headers);
  $request .= "Content-Length: $length\r\n" if $length;
  print $up $request . "Connection: close\r\n\r\n";
  if ($length) {
    # read_head 可能连请求体的开头一起读进来了，先补它，再从连接读剩下的。
    my $first = substr($rest // '', 0, $length);
    print $up $first;
    copy($client, $up, $length - length $first);
  }

  my ($status, $type, $chunked, $out, $rrest, $gz) = read_response_head($up);
  unless ($status) { close $up; close $client; return; }

  # 保险：拿回来的 HTML 若是压缩的（只有没挡掉压缩的 XHR 会这样），按字节插会把页面插坏，原样放行。
  my $html = $nav && !$gz && $type =~ m{^text/html}i && $status =~ /^HTTP\/1\.[01] 200/;
  if ($html) {
    my $raw = $rrest // '';
    $raw .= do { local $/; <$up> } // '';
    my $body = $chunked ? dechunk($raw) : $raw;
    $body =~ s{</head>}{<style id="wonton-layout">\n$css</style>\n</head>}i
      or $body =~ s{(<body[^>]*>)}{$1\n<style id="wonton-layout">\n$css</style>}i;
    my $head = "$status\r\n" . join('', map { "$_\r\n" } grep { !/^(content-length|transfer-encoding|etag):/i } @$out);
    $head .= 'ETag: W/"' . length($body) . "-wonton\"\r\n";
    $head .= 'Content-Length: ' . length($body) . "\r\n" unless $status =~ /(?:204|304)\b/;
    print $client $head . "Connection: close\r\n\r\n";
    print $client $body unless $method eq 'HEAD';
  } else {
    my $head = "$status\r\n" . join('', map { "$_\r\n" } @$out);
    print $client $head . "Connection: close\r\n\r\n";
    print $client $rrest if defined $rrest && length $rrest;
    while (1) {
      my $n = sysread($up, my $buf, 65536);
      last unless $n;
      print $client $buf;
    }
  }
  close $up;
  close $client;
}

# (方法, 路径, 转发的请求头, Content-Length, 多读到的请求体开头)；请求不合格时 undef。
sub read_request {
  my ($client) = @_;
  my ($head, $rest) = read_head($client) or return;
  my ($start, @lines) = split /\r?\n/, $head;
  $start =~ s/\r$//;
  my ($method, $path) = split / /, $start;
  return unless defined $path;
  # 导航请求要拿注入过的页面，所以要挡掉条件请求：上游的 ETag 算在注入前的正文上，
  # 顺着它会回 304，浏览器就用回旧页面，注入的新样式看不到。Accept 不一定排在
  # If-None-Match 前面，先整段看一眼。
  my $nav = grep { /^accept:.*text\/html/i } @lines;
  my ($length, @keep);
  for my $line (@lines) {
    $line =~ s/\r$//;
    next if $line eq '';
    my ($name) = split /:/, $line;
    # Host 和 Expect 由这里应掉：Host 由代理按上游重写，请求体是先读完才转发的，传 Expect 只会
    # 叫客户端白等一个 100 Continue。
    next if lc $name =~ /^(host|connection|keep-alive|proxy-connection|expect)$/;
    next if $nav && lc $name eq 'accept-encoding';
    next if $nav && lc $name =~ /^(if-none-match|if-modified-since)$/;
    $length = $1 if $line =~ /^content-length:\s*(\d+)/i;
    push @keep, $line;
  }
  return ($method, $path, \@keep, $length, $rest, $nav);
}

# (状态行, Content-Type, 是否 chunked, 转发的响应头, 多读到的响应体开头, 是否压缩)
sub read_response_head {
  my ($up) = @_;
  my ($head, $rest) = read_head($up) or return;
  my ($status, @lines) = split /\r?\n/, $head;
  $status =~ s/\r$//;
  my ($type, $chunked, $gz, @keep) = ('', 0, 0);
  for my $line (@lines) {
    $line =~ s/\r$//;
    next if $line eq '';
    my ($name) = split /:/, $line;
    next if lc $name =~ /^(connection|keep-alive)$/;
    $type = $1 if $line =~ /^content-type:\s*(.*)/i;
    $chunked = 1 if lc $name eq 'transfer-encoding' && $line =~ /chunked/i;
    $gz = 1 if lc $name eq 'content-encoding' && $line =~ /gzip|deflate|br|zstd/i;
    push @keep, $line;
  }
  return ($status, $type, $chunked, \@keep, $rest, $gz);
}

# (头, 多读到的正文开头)。一次 sysread 可能把 \r\n\r\n 之后的内容也读进来，
# 丢掉的话请求体和响应体都会少一段。
sub read_head {
  my ($sock) = @_;
  my $buf = '';
  while (length $buf < 65536) {
    my $n = sysread($sock, $buf, 4096, length $buf);
    return undef unless $n;
    if ($buf =~ /\r?\n\r?\n/) {
      my ($head, $rest) = split /\r?\n\r?\n/, $buf, 2;
      return ($head, $rest // '');
    }
  }
  undef
}

# 网页的 HTML 不大，整页读完再按块长度拆开。
sub dechunk {
  my ($raw) = @_;
  my $out = '';
  while (1) {
    my $i = index($raw, "\n");
    last if $i < 0;
    my $line = substr($raw, 0, $i);
    $line =~ s/\r$//;
    my ($size) = $line =~ /^([0-9a-fA-F]+)/ or last;
    $size = hex $size;
    substr($raw, 0, $i + 1) = '';
    last if $size == 0;
    $out .= substr($raw, 0, $size);
    substr($raw, 0, $size) = '';
    $raw =~ s/^\r?\n//;
  }
  $out
}

sub copy {
  my ($from, $to, $length) = @_;
  my $left = $length;
  while ($left > 0) {
    my $n = sysread($from, my $buf, $left < 65536 ? $left : 65536) or last;
    print $to $buf;
    $left -= $n;
  }
}
