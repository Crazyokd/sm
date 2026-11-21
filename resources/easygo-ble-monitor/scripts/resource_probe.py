import os,time,json,pathlib,sys
wanted={'peer-information','peer_geometry_publisher','component_container_isolated','bluetoothd','easygo-app'}
hz=os.sysconf('SC_CLK_TCK');page=os.sysconf('SC_PAGE_SIZE');start=time.monotonic();old={}
duration=int(sys.argv[1]) if len(sys.argv)>1 else 90
for i in range(duration+1):
 now=time.monotonic();out={'epoch_ns':time.time_ns(),'elapsed':now-start,'cpu_count':os.cpu_count(),'loadavg':os.getloadavg(),'processes':[]}
 for folder in pathlib.Path('/proc').iterdir():
  if not folder.name.isdigit():continue
  try:
   name=os.path.basename((folder/'cmdline').read_bytes().split(b'\0')[0].decode())
   if name not in wanted:continue
   pid=int(folder.name);raw=(folder/'stat').read_text();v=raw[raw.rindex(')')+2:].split();ticks=int(v[11])+int(v[12]);rss=int(v[21])*page;cpu=None;key=(pid,int(v[19]))
   if key in old:cpu=(ticks-old[key][0])/hz/(now-old[key][1])*100
   old[key]=(ticks,now)
   try:fds=len(list((folder/'fd').iterdir()))
   except PermissionError:fds=None
   out['processes'].append({'pid':pid,'name':name,'cpu_percent_one_core':cpu,'rss_bytes':rss,'threads':int(v[17]),'fds':fds})
  except (OSError,ValueError,UnicodeError):pass
 out['unavailable']=sorted(wanted-{p['name'] for p in out['processes']})
 out['cpu_pressure']=pathlib.Path('/proc/pressure/cpu').read_text().strip() if pathlib.Path('/proc/pressure/cpu').exists() else None;out['meminfo']={l.split(':')[0]:l.split(':')[1].strip() for l in pathlib.Path('/proc/meminfo').read_text().splitlines() if l.startswith(('MemTotal:','MemAvailable:','SwapTotal:','SwapFree:'))};print(json.dumps(out),flush=True)
 if i<duration:time.sleep(max(0,start+i+1-time.monotonic()))
