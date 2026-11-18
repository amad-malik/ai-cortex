"""Whole fake installer: close stdout only after an unpublished stage exists."""
from pathlib import Path
import argparse,fcntl,hashlib,json,os,pty,select,signal,subprocess,termios,time
p=argparse.ArgumentParser();p.add_argument('source');p.add_argument('home');p.add_argument('out');p.add_argument('kind',choices=['pipe','pty']);p.add_argument('cause',choices=['HUP','STOP']);a=p.parse_args()
out=Path(a.out);out.mkdir(mode=0o700);state=Path(os.environ['FAKE_STATE']);e=os.environ.copy();e.update(FAKE_BOUNDARY_PHASE='dead-stdout',FAKE_FIXED_STAMP='1',FAKE_BOOT_NO_NEWLINE='1',FAKE_ABORT_PRINT_BARRIER='1',CORTEX_LOG=str(out/'application.log'))
value=e['FAKE_BOOT_TOKEN'].encode();old_values=[f.read_bytes().strip() for f in Path(a.home).glob('*.token')];before={f.name:hashlib.sha256(f.read_bytes()).hexdigest() for f in Path(a.home).glob('*.token')};reader,writer=os.pipe() if a.kind=='pipe' else pty.openpty();stderr=(out/'stderr').open('wb');captured=bytearray();argv=[]
def child_setup():
 os.setsid()
 if a.kind=='pty':fcntl.ioctl(writer,termios.TIOCSCTTY,0)
q=subprocess.Popen(['sh','-x',a.source,'--project','p','--lead','lead','--home',a.home,'--reissue-token'],stdin=subprocess.DEVNULL,stdout=writer,stderr=stderr,env=e,preexec_fn=child_setup)
os.close(writer);os.set_blocking(reader,False);deadline=time.monotonic()+45;observed=False;closed=False;signal_sent=False;stage_witness=[];print_observed=False;cleanup_before_print=False;release_held_before_print=False;stage_real_value=False
def sample_argv():
 todo=[q.pid];seen=set()
 while todo:
  pid=todo.pop()
  if pid in seen:continue
  seen.add(pid)
  try:
   if os.getpgid(pid)!=q.pid:continue
   argv.append(Path('/proc',str(pid),'cmdline').read_bytes())
   todo.extend(int(x) for x in Path('/proc',str(pid),'task',str(pid),'children').read_text().split())
  except (FileNotFoundError,ProcessLookupError,PermissionError):pass
try:
 while q.poll() is None:
  if time.monotonic()>deadline:raise TimeoutError('owned abort-output installer exceeded45s')
  sample_argv()
  if not observed and (state/'dead-stdout-ready').exists():
   stage_witness=[str(f) for f in Path(a.home).glob('*.part.*') if f.is_file() and f.stat().st_size]
   observed=bool(stage_witness) and (state/'bootstrap.token').is_file() and (Path(e['SUM_ACCOUNT_RUNTIME'])/'cortex-mint.reservation').is_dir()
   if not observed:raise AssertionError('boundary must contain stage, server copy and reservation')
   stage_real_value=all(Path(f).read_bytes()==value and (Path(f).stat().st_mode & 0o777)==0o600 and f.endswith('.part.'+str(q.pid)) for f in stage_witness)
   while True:
    try:data=os.read(reader,65536)
    except BlockingIOError:break
    except OSError:break
    if not data:break
    captured.extend(data)
    assert len(captured)<=1048576,'pre-close stdout bound'
   os.close(reader);closed=True
   if a.cause=='HUP':
    # Closing a controlling PTY also sends HUP; explicit delivery gives both
    # stdout kinds the identical first signal contract.
    if a.kind=='pipe':os.kill(q.pid,signal.SIGHUP)
    signal_sent=True  # the controlling PTY delivers its own single HUP
   (state/'dead-stdout-go').touch()
  if not print_observed and (state/'abort-print-ready').exists():
   print_observed=True
   cleanup_before_print=not list(Path(a.home).glob('*.part.*')) and not (state/'bootstrap.token').exists()
   release_held_before_print=(Path(e['SUM_ACCOUNT_RUNTIME'])/'cortex-mint.reservation').is_dir()
   (state/'abort-print-go').touch()
  if not closed and select.select([reader],[],[],0.02)[0]:
   try:data=os.read(reader,65536)
   except OSError:data=b''
   captured.extend(data)
  else:time.sleep(0.01)
 rc=q.wait(timeout=2)
finally:
 if q.poll() is None:
  os.killpg(q.pid,signal.SIGKILL);q.wait()
 if not closed:os.close(reader)
 stderr.close()
(out/'stdout-before-close').write_bytes(captured);(out/'argv').write_bytes(b'\n'.join(argv))
streams=[bytes(captured),(out/'stderr').read_bytes(),(out/'argv').read_bytes()]
for f in (out/'application.log',Path(e['FAKE_LOG'])):
 if f.exists():streams.append(f.read_bytes())
private=all(secret not in data and all(secret[i:i+16] not in data for i in range(len(secret)-15)) for data in streams for secret in [value,*old_values])
left=[str(f) for f in Path(a.home).glob('*.part.*')];reservation=(Path(e['SUM_ACCOUNT_RUNTIME'])/'cortex-mint.reservation').exists();copy=(state/'bootstrap.token').exists();old_preserved=all((Path(a.home)/n).is_file() and hashlib.sha256((Path(a.home)/n).read_bytes()).hexdigest()==v for n,v in before.items())
want=129 if a.cause=='HUP' or a.kind=='pty' else 7
checks={'boundary_observed':observed,'real_length_newline_free_stage':stage_real_value,'abort_print_observed':print_observed,'cleanup_before_print':cleanup_before_print,'release_still_held_at_print':release_held_before_print,'argv_and_trace_observed':bool(argv) and bool((out/'stderr').stat().st_size),'status':rc==want,'no_stage':not left,'no_server_copy':not copy,'no_reservation':not reservation,'old_token_preserved':old_preserved,'all_stream_value_and_fragments_private':private}
report={'kind':a.kind,'cause':a.cause,'native':rc,'wanted':want,'signal_sent':signal_sent,'checks':checks,'stage_witness':stage_witness,'left_stage':left,'source_sha256':hashlib.sha256(Path(a.source).read_bytes()).hexdigest(),'scope':'whole private fake installer; own dead stdout and process session; no real credentials or container'}
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report));raise SystemExit(0 if all(checks.values()) else 1)
