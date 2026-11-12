import json,subprocess as s,time
U="F6EF9B7F-9F73-4933-9085-A5CEED038838";P="/Users/jiale/Library/Python/3.14/bin/idb"
def R(*a):return s.run(a,capture_output=True,text=True).stdout
def D():
 try:return json.loads(R(P,"ui","describe-all","--udid",U))
 except:return[]
def T(x,y):R(P,"ui","tap","--udid",U,str(x),str(y))
def F(n,role="StaticText"):
 for e in D():
  if n in (e.get("AXLabel") or "") and e.get("type")==role and (e.get("frame") or {}).get("width"):
   f=e["frame"];return int(f["x"]+f["width"]/2),int(f["y"]+f["height"]/2)
R("xcrun","simctl","launch",U,"app.wonton.ios");time.sleep(1)
for i in range(3):R(P,"ui","swipe","--udid",U,"200","700","200","200","--duration","0.3")
time.sleep(1)
p=F("mailto:test@example.com") or F("再看一行");print("pos",p,flush=True)
if p:
 R(P,"ui","tap","--udid",U,str(p[0]),str(p[1]),"--duration","1.3");time.sleep(1.5)
 L=[e.get("AXLabel") for e in D() if e.get("AXLabel")]
 print("menu",L[:8],flush=True)
 c=F("复制")
 if c:
  T(*c);time.sleep(1);print("clip",repr(R("xcrun","simctl","pbpaste",U))[:60],flush=True)
if not c:
 R(P,"ui","swipe","--udid",U,"400","400","100","400","--duration","0.3");time.sleep(1)
 R("xcrun","simctl","io",U,"screenshot","/Users/jiale/wonton-dev/y.png")
o=R("/opt/homebrew/bin/magick","/Users/jiale/wonton-dev/y.png","-fx","(b>r+0.08&&b>0.3)?1:0","-format","%@","info:").strip()
print("bbox",o,flush=True)
