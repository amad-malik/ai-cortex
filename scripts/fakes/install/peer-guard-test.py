"""Full fake installer and an actual synthetic loopback whoami listener."""
from pathlib import Path
import argparse,http.server,hashlib,json,os,signal,subprocess,threading
p=argparse.ArgumentParser();p.add_argument('source');p.add_argument('home');p.add_argument('out');p.add_argument('mode',choices=['repeat-exited','repeat-db','summary-exited','summary-db-exited','repeat-port','repeat-unavailable','repeat-race-server','repeat-race-port','positive']);a=p.parse_args();out=Path(a.out);out.mkdir(mode=0o700);requests=[];token=os.environ['FAKE_BOOT_TOKEN'];state=Path(os.environ['FAKE_STATE'])
class Listener(http.server.BaseHTTPRequestHandler):
 def log_message(self,*args):pass
 def do_GET(self):
  auth=self.headers.get('Authorization','');requests.append({'path':self.path,'bearer_present':auth.startswith('Bearer '),'expected_synthetic_value':auth=='Bearer '+token})
  body=json.dumps({'name':'lead','kind':'worker','role':'project_lead','project':'11111111-1111-4111-8111-111111111111'}).encode();self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Listener);thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start();port=server.server_port;e=os.environ.copy()
for key in ('FAKE_DB_EXITED','FAKE_DB_HEALTH','FAKE_SERVER_EXITED','FAKE_PORT_WRONG','FAKE_PORT_FAIL','FAKE_PEER_RACE','FAKE_REAL_WHOAMI_LISTENER'):e.pop(key,None)
command=['sh',a.source,'--project','p','--lead','lead','--home',a.home,'--port',str(port),'--no-show-credentials'];runs=[]
def run(label,env):
 env=dict(env,CORTEX_LOG=str(out/(label+'.application.log')))
 with (out/(label+'.stdout')).open('wb') as so,(out/(label+'.stderr')).open('wb') as se:
  q=subprocess.Popen(command,env=env,stdout=so,stderr=se,start_new_session=True)
  try:rc=q.wait(timeout=45)
  except subprocess.TimeoutExpired:
   os.killpg(q.pid,signal.SIGKILL);q.wait();raise
 runs.append({'label':label,'native':rc});return rc
try:
 setup=0
 if not a.mode.startswith('summary-'):setup=run('setup',e)
 assert setup==0,'healthy setup failed'
 env=dict(e,FAKE_REAL_WHOAMI_LISTENER='1')
 if a.mode=='summary-db-exited':env['FAKE_DB_EXITED']='1'
 elif a.mode=='repeat-db':env['FAKE_DB_HEALTH']='starting'
 elif a.mode.endswith('exited'):env['FAKE_SERVER_EXITED']='1'
 elif a.mode=='repeat-port':env['FAKE_PORT_WRONG']='1'
 elif a.mode=='repeat-unavailable':env['FAKE_PORT_FAIL']='1'
 elif a.mode=='repeat-race-server':env['FAKE_PEER_RACE']='server'
 elif a.mode=='repeat-race-port':env['FAKE_PEER_RACE']='port'
 status=run('subject',env)
finally:server.shutdown();server.server_close();thread.join(timeout=2)
public=[]
for f in out.iterdir():
 if f.is_file():public.append(f.read_bytes())
if Path(e['FAKE_LOG']).exists():public.append(Path(e['FAKE_LOG']).read_bytes())
value=token.encode();private=all(value not in data and all(value[i:i+16] not in data for i in range(len(value)-15)) for data in public)
bearers=sum(x['bearer_present'] for x in requests);positive=a.mode=='positive';checks={'listener_received_expected_request_if_positive':bool(bearers and all(x['expected_synthetic_value'] for x in requests)) if positive else bearers==0,'status':status==0 if positive else status!=0,'synthetic_value_private_on_public_streams':private}
report={'mode':a.mode,'port':port,'runs':runs,'requests':requests,'checks':checks,'source_sha256':hashlib.sha256(Path(a.source).read_bytes()).hexdigest(),'scope':'actual owned loopback listener, synthetic bearer; fake containers; no real account/network service'};(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report));raise SystemExit(0 if all(checks.values()) else 1)
