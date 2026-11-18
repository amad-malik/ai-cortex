from pathlib import Path
import argparse,errno,fcntl,hashlib,json,os,pty,re,select,shutil,signal,sys,time
p=argparse.ArgumentParser();p.add_argument('source');p.add_argument('home');p.add_argument('out');p.add_argument('mode',choices=['tail','tail-error','tail-rearm','head-normal','head-signal','head-error','head-raw']);p.add_argument('sig',choices=['SKIP','HUP','INT','TERM']);a=p.parse_args()
out=Path(a.out);out.mkdir(parents=True,exist_ok=False);deploy=out/'deploy';shutil.copytree(Path(a.source).parent,deploy)
s=(deploy/'install.sh').read_text();original_hash=hashlib.sha256(s.encode()).hexdigest();pad=b'~'*262144
if a.mode in ('head-normal','tail-rearm'):
 i=s.rfind('summary_finish\n');assert i>=0;s=s[:i]+s[i:].replace('summary_finish\n','summary_finish\nset -x\n',1)
elif a.mode=='head-signal':
 marker='  ln -T -- "$part" "$dest" 2>/dev/null';assert s.count(marker)==1;s=s.replace(marker,'  set -x\n'+marker,1)
elif a.mode=='head-error':
 marker='log_line "install: $*"; ';assert s.count(marker)==1;s=s.replace(marker,marker+'set -x; ',1)
elif a.mode=='head-raw':
 marker='  mint_token "$token"\n';assert s.count(marker)==1;s=s.replace(marker,marker+'  set -x\n  false\n',1)
# Trace uses FD4 so public stderr remains independently comparable. Mirror the
# shipped grouped stderr suppression onto that private trace descriptor too;
# otherwise the separate channel would invent a blocking write at each group.
s=re.sub(r'(\{ [^{}\n]+; \} 2>/dev/null)', r'\1 4>/dev/null', s)
(deploy/'install.sh').write_text(s)
if a.mode=='tail-rearm':
 target_at=s.index('  arm_final_signal_traps\n') if '  arm_final_signal_traps\n' in s else s.index('  if [ "$eh_st" = 0 ] && [ -n "$exit_handler_signal" ]')
elif a.mode.startswith('tail'):
 target_at=s.index('  : # tracing is restored') if '  : # tracing is restored' in s else s.index('  exit "$eh_st"')
elif a.mode in ('head-normal','head-raw'):
 target_at=s.index('  eh_st=',s.index('exit_handler() {'))
elif a.mode=='head-signal':
 target_at=s.index('exit "$1"',s.index('install_signal() {'))
else:
 target_at=s.index('fail() {')
target_line=s[:target_at].count('\n')+1
ps4='__TRACE__${FUNCNAME:-main}:${LINENO}__END__'+pad.decode()+' '

(deploy/'install-prereq.sh').write_text("PS4='"+ps4+"'\n")
e=os.environ.copy();e['BASH_XTRACEFD']='4'
if a.mode in ('tail-error','head-error'):e['FAKE_SYSTEMCTL_FAIL']='1'
if a.mode=='head-signal':e['FAKE_HEAD_FIRST_SIGNAL']='1'
r,w=os.pipe();capacity=fcntl.fcntl(w,fcntl.F_GETPIPE_SZ);assert capacity<len(pad)//2
pid,tty=pty.fork()
if pid==0:
 os.close(r)
 fd=os.open(out/'stdout',os.O_WRONLY|os.O_CREAT|os.O_TRUNC,0o600);er=os.open(out/'stderr',os.O_WRONLY|os.O_CREAT|os.O_TRUNC,0o600)
 os.dup2(fd,1);os.dup2(er,2);os.dup2(w,4);os.set_inheritable(4,True)
 for old in {fd,er,w}-{1,2,4}:os.close(old)
 os.execvpe('sh',['sh','-x',str(deploy/'install.sh'),'--project','p','--lead','lead','--home',a.home],e)
os.close(w);os.set_blocking(r,False);os.set_blocking(tty,False)
compact=bytearray();pending=bytearray();tty_bytes=bytearray();skip=0;delivered=False;observed=False;status=None;end=time.monotonic()+60
start=b'__TRACE__';finish=b'__END__'
seen=[]
def matches(marker):
 text=marker.decode('ascii','replace')
 if re.fullmatch(r'[A-Za-z_]+:[0123456789]+',text) and len(seen)<30:seen.append(text)
 want='exit_handler' if a.mode.startswith('tail') or a.mode in ('head-normal','head-raw') else 'install_signal' if a.mode=='head-signal' else 'fail'
 return text==want+':'+str(target_line)
def consume(data,final=False):
 global skip,delivered,observed
 pending.extend(data.replace(b'~',b''))
 while pending:
  pos=pending.find(start)
  if pos<0:
   n=len(pending) if final else max(0,len(pending)-len(start));compact.extend(pending[:n]);del pending[:n];break
  compact.extend(pending[:pos]);del pending[:pos]
  stop=pending.find(finish,len(start))
  if stop<0:break
  marker=bytes(pending[len(start):stop]);stop+=len(finish)
  matched=matches(marker)
  if not observed and matched:
   observed=True
   if a.sig!='SKIP':os.kill(pid,getattr(signal,'SIG'+a.sig));delivered=True
  compact.extend(pending[:stop]);del pending[:stop];skip=0
  assert len(compact)<16777216,'compact trace bound'
open_fds={r,tty}
try:
 while open_fds or status is None:
  if time.monotonic()>end:
   os.killpg(pid,signal.SIGKILL);raise TimeoutError('owned probe exceeded60s')
  ready,_,_=select.select(list(open_fds),[],[],.05)
  for fd in ready:
   try:data=os.read(fd,4096)
   except OSError as ex:
    if ex.errno==errno.EIO:data=b''
    else:raise
   if not data:open_fds.remove(fd);os.close(fd);continue
   if fd==r:consume(data)
   else:tty_bytes.extend(data);assert len(tty_bytes)<65536,'tty bound'
  if status is None:
   got,st=os.waitpid(pid,os.WNOHANG)
   if got:status=st
 consume(b'',True);assert not pending and skip==0,'partial trace'
finally:
 for fd in open_fds:
  try:os.close(fd)
  except OSError:pass
 if status is None:
  try:os.killpg(pid,signal.SIGKILL);_,status=os.waitpid(pid,0)
  except ProcessLookupError:pass
raw=bytes(compact);(out/'trace').write_bytes(raw)
# Strip only command tracing after collecting the raw compact stream. Padding
# uses '~', outside credential grammar; all value/fragment checks see raw bytes.
public=(out/'stderr').read_bytes()
(out/'public-stderr').write_bytes(public)
value=e.get('FAKE_BOOT_TOKEN','').encode();guard=True
for public_data in ((out/'stdout').read_bytes(),raw,public):
 for view in (public_data,public_data.replace(b'\r',b'').replace(b'\n',b'')):
  if value and (value in view or any(value[i:i+16] in view for i in range(len(value)-15))):guard=False
rc=os.waitstatus_to_exitcode(status)
report={'mode':a.mode,'signal':a.sig,'status':rc,'boundary_observed':observed,'signal_delivered':delivered,'raw_value_guard':guard,'tty_value_count':tty_bytes.replace(b'\r',b'').splitlines().count(value) if value else 0,'problem_count':len(re.findall(rb'^problem: ',public,re.M)),'fix_count':len(re.findall(rb'^fix: ',public,re.M)),'origin_sha256':original_hash,'instrumented_sha256':hashlib.sha256(s.encode()).hexdigest(),'pipe_capacity':capacity,'target_line':target_line,'markers_seen':seen,'padding':len(pad),'scope':'owned fake installer/pty only; private source trace instrumentation; no real credentials or runtime'}
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report))
raise SystemExit(0 if observed and guard and (a.sig=='SKIP' or delivered) else 1)
