"""Render an offline 1080p replay from prepared.json using Pillow and ffmpeg."""
import argparse
from bisect import bisect_right
from functools import lru_cache
import json
import math
from pathlib import Path
import shutil
import subprocess
import sys

sys.dont_write_bytecode = True

from PIL import Image, ImageDraw, ImageFont
from prepare import advice_at, completion_at, source

BG, INK, MUTED, PANEL, FLOOR = '#101827', '#f0f5fa', '#a6b5c8', '#1b283a', '#f7f8f4'
COLORS = {1: '#078eaf', 2: '#db8b13'}
REC = {0: ('无当前建议', '#8a98ad'), 1: ('继续 CONTINUE', '#37c5b1'), 2: ('等待 WAIT', '#ffce72'), 3: ('让行 YIELD', '#c5a5ff')}
W, H = 1920, 1080


@lru_cache(None)
def font(size, bold=False):
    query = 'Noto Sans CJK SC' + (':weight=bold' if bold else '')
    values = subprocess.check_output(['fc-match', '-f', '%{file}\n%{index}\n', query], text=True).splitlines()
    face=ImageFont.truetype(values[0], size, index=int(values[1]))
    a,b=face.getmask('回'),face.getmask('放')
    if (a.size,bytes(a))==(b.size,bytes(b)):
        raise ValueError('Noto Sans CJK SC with Chinese glyphs is required; font fallback produced missing-glyph boxes')
    return face


def text(draw, xy, value, size=25, color=INK, bold=False, width=None):
    value = str(value).replace('\n', ' ')
    if width is not None:
        while value and draw.textlength(value, font=font(size, bold)) > width:
            value = value[:-2] + '…'
    draw.text(xy, value, font=font(size, bold), fill=color)


def panel(draw, box, color=PANEL, radius=16):
    draw.rounded_rectangle(box, radius=radius, fill=color)


def cover(manifest, ending=False):
    image = Image.new('RGB', (W, H), BG);d = ImageDraw.Draw(image)
    scenes = manifest['scenes'];passed = sum(s['passed'] for s in scenes)
    text(d, (96,70), '多车协作 · 验收数据回放', 60, bold=True)
    text(d, (100,176), manifest['source_label'], 34, '#64d7cd', width=1700)
    text(d, (100,260), '真实物理位姿、协作建议和原任务终态分别显示', 30, MUTED)
    for i, scene in enumerate(scenes[:4]):
        x, y = 100+(i%2)*850, 368+(i//2)*168
        panel(d, (x,y,x+800,y+132))
        text(d, (x+28,y+25), f'{i+1:02d}   {scene["title"]}', 36, bold=True, width=740)
        text(d, (x+28,y+78), '原回执：通过' if scene['passed'] else '原回执：未通过', 25, '#71d3a9' if scene['passed'] else '#ff9b8e')
    text(d, (100,758), f'本片共 {len(scenes)} 个场景；原回执通过 {passed}/{len(scenes)}' if ending else '蓝色：机器人 1   橙色：机器人 2   实线：已行轨迹', 32)
    text(d, (100,825), '回放不构成一轮新的验收；未展示的运行仍以各自原回执为准。', 27, MUTED)
    text(d, (100,918), '本片也包含未通过轮次。' if passed != len(scenes) else '选取通过轮次不代表每次运行都成功；原失败证据应继续保留。', 25, '#d9b97c')
    text(d, (100,994), '原始采样保留在来源记录中；画面按显示帧率抽样，非实时录屏。', 23, MUTED)
    return image


class Scene:
    def __init__(self, manifest, index, speed):
        self.m, self.s, self.index = manifest, manifest['scenes'][index], index
        self.a = {i:self.s['poses'][str(i)] for i in (1,2)}
        self.times = {i:[r[0] for r in self.a[i]] for i in (1,2)}
        self.advice = {i:self.s['advice'][str(i)] for i in (1,2)}
        self.box = (36,180,1432,924)
        xmin,xmax,ymin,ymax = self.s['bounds']
        self.scale = min(1352/(xmax-xmin),700/(ymax-ymin))
        self.xmin, self.ymax = xmin, ymax
        self.ox = 36+(1396-(xmax-xmin)*self.scale)/2
        self.oy = 180+(744-(ymax-ymin)*self.scale)/2
        self.pix = {i:[self.xy(r[1],r[2]) for r in self.a[i]] for i in (1,2)}
        self.base = Image.new('RGB',(W,H),BG);d=ImageDraw.Draw(self.base)
        text(d,(36,24),f'{index+1:02d} / {len(manifest["scenes"]):02d}   {self.s["title"]}',49,bold=True,width=1380)
        text(d,(38,100),'原回执：'+('通过' if self.s['passed'] else '未通过')+' · 真实记录回放 · 原目标保持',26,'#71d3a9' if self.s['passed'] else '#ff9b8e')
        panel(d,(1510,38,1884,116),'#24364c');text(d,(1532,49),f'{speed:g} 倍速回放',31,'#87d8eb',True)
        layer=Image.new('RGB',(1396,744),FLOOR);ld=ImageDraw.Draw(layer)
        for x in range(math.floor(xmin),math.ceil(xmax)+1):
            px=self.xy(x,0)[0]-36;ld.line((px,0,px,744),fill='#e6eae4')
        for y in range(math.floor(ymin),math.ceil(ymax)+1):
            py=self.xy(0,y)[1]-180;ld.line((0,py,1396,py),fill='#e6eae4')
        for wall in manifest['walls']:
            ld.polygon([(self.xy(x,y)[0]-36,self.xy(x,y)[1]-180) for x,y in wall],fill='#566272')
        self.base.paste(layer,(36,180));d=ImageDraw.Draw(self.base)
        for i in (1,2):
            goal=self.s['goals'][str(i)];gx,gy=self.xy(goal['x'],goal['y'])
            d.ellipse((gx-12,gy-12,gx+12,gy+12),outline=COLORS[i],width=3)
            d.line((gx,gy-12,gx,gy-96),fill=COLORS[i],width=2)
            panel(d,(gx-55,gy-135,gx+55,gy-96),'#ffffff',8);text(d,(gx-43,gy-133),f'终点 {i}',22,COLORS[i],True)
            sy=180+(i-1)*190;panel(d,(1460,sy,1884,sy+172))
            d.ellipse((1482,sy+21,1528,sy+67),fill=COLORS[i]);text(d,(1497,sy+21),i,25,INK,True);text(d,(1546,sy+20),f'机器人 {i}',31,bold=True)
        panel(d,(54,195,450,244),'#ffffff');text(d,(70,200),'实线：已行轨迹   箭头：车头',21,'#405064')
        panel(d,(1460,574,1884,924));text(d,(1484,592),'该轮原回执指标',28,bold=True)
        physics=self.s['physics'] or {};bodies=physics.get('single_vehicle',{})
        wall=[v['minimum_continuous_bound_m'] for v in bodies.values() if v.get('minimum_continuous_bound_m') is not None]
        gap=[v['maximum_sample_gap_s'] for v in bodies.values() if v.get('maximum_sample_gap_s') is not None]
        pair=physics.get('minimum_pair_continuous_bound_m');rtf=self.s['real_time_factor']
        rows=[('车—墙连续净空下界',f'{min(wall)*1000:.1f} mm' if wall else '未记录'),('两车连续净空下界',f'{pair*1000:.1f} mm' if pair is not None else '未记录'),('最大物理采样间隔',f'{max(gap)*1000:.1f} ms' if gap else '未记录'),('实时率',f'{rtf:.3f}' if rtf is not None else '未记录')]
        for j,(name,value) in enumerate(rows):
            yy=642+j*62;text(d,(1484,yy),name,21,MUTED);text(d,(1484,yy+24),value,25,bold=True)
        text(d,(38,1030),'来源：'+manifest['source_label']+' | 原始物理采样 + 协作记录 + 终态锚点 | 非实时录屏',20,MUTED,width=1840)

    def xy(self,x,y):
        return self.ox+(x-self.xmin)*self.scale,self.oy+(self.ymax-y)*self.scale

    def frame(self,t,lead=False):
        image=self.base.copy();d=ImageDraw.Draw(image);records={};advice={};done={}
        for i in (1,2):
            index=max(0,bisect_right(self.times[i],t)-1);records[i]=self.a[i][index]
            points=self.pix[i][:index+1:10]
            if len(points)>1:d.line(points,fill=COLORS[i],width=3)
            rec,stamp=advice_at(self.advice[i],t)
            advice[i]=stamp,rec
            terminal=self.s['terminal_s'][str(i)];done[i]=completion_at(terminal,t)
        for i in (1,2):
            r=records[i];x,y,a=r[1:4];c,s=math.cos(a),math.sin(a)
            d.polygon([self.xy(x+c*px-s*py,y+s*px+c*py) for px,py in self.m['body_polygon']],fill=COLORS[i],outline='#1e3544',width=2)
            outline=self.m['body_polygon'];cx=(min(p[0] for p in outline)+max(p[0] for p in outline))/2;cy=(min(p[1] for p in outline)+max(p[1] for p in outline))/2
            length=max(p[0] for p in outline)-min(p[0] for p in outline);width=max(p[1] for p in outline)-min(p[1] for p in outline)
            arrow=[(cx+.3*length,cy),(cx-.04*length,cy+.18*width),(cx-.04*length,cy-.18*width)]
            d.polygon([self.xy(x+c*px-s*py,y+s*px+c*py) for px,py in arrow],fill='#ffffff')
            px,py=self.xy(x,y);by=py-37 if i==1 else py+29
            d.ellipse((px-13,by-13,px+13,by+13),fill=COLORS[i],outline='#ffffff');text(d,(px-6,by-16),i,20,INK,True)
            sy=180+(i-1)*190;motion='行驶' if r[4]>.01 else ('转向' if r[5]>.02 else '静止')
            text(d,(1484,sy+65),f'{motion}   {r[4]:.2f} m/s',26,bold=True)
            stamp,rec=advice[i]
            if done[i]:label,color='原任务已完成','#71d3a9';note=f'终态锚点 {self.s["terminal_s"][str(i)]:.1f} s'
            elif stamp is None:label,color,note='建议：未记录',MUTED,'没有早于此刻的建议记录'
            elif rec is None:label,color,note='暂无可用建议',MUTED,f'最近记录于 {stamp:.1f} s'
            else:label,color=REC[rec];label='建议：'+label;note=f'最近记录于 {stamp:.1f} s'
            if done[i] is None:note='终态未知；'+note
            text(d,(1484,sy+103),label,23,color);text(d,(1484,sy+139),note,17,MUTED,width=375)
        caption='同色圆环为原目标，实线为已经发生的真实运动。'
        if all(done.values()):caption='两车终态锚点均已到达；该轮判定以原验收回执为准。'
        elif any(value is None for value in done.values()):caption='存在未记录的原任务完成锚点；完成时刻保持未知。'
        elif any(done.values()):caption='一台车的原任务已完成，另一台仍未到达其终态锚点。'
        elif any(a[1]==3 for a in advice.values()):caption='最近记录含让行建议；实际运动见左侧轨迹与右侧速度。'
        elif any(a[1]==2 for a in advice.values()):caption='最近记录含等待建议；建议和实际停稳状态分开显示。'
        panel(d,(54,862,1414,910),'#ffffff');text(d,(72,866),caption,25,'#24364c')
        text(d,(40,946),f'本段仿真时间  {t:06.1f} / {self.s["duration_s"]:.1f} s',26)
        start=0 if len(self.m['scenes'])<=4 else max(0,self.index-1)
        for j,s in enumerate(self.m['scenes'][start:start+4]):
            xx=1065+j*203;active=start+j==self.index
            panel(d,(xx,949,xx+184,996),'#284659' if active else PANEL,10);text(d,(xx+12,953),s['title'],23,'#8ae1d5' if active else MUTED,active,width=160)
        d.rectangle((40,998,1010,1003),fill='#2b3b4f');d.rectangle((40,998,40+970*t/self.s['duration_s'],1003),fill='#64d7cd')
        if lead:
            panel(d,(400,460,1070,570));text(d,(430,475),self.s['title'],38,bold=True,width=610)
        return image


def verify_video(path):
    path=Path(path).resolve()
    probe=json.loads(subprocess.check_output(['ffprobe','-v','error','-show_streams','-show_format','-show_chapters','-of','json',str(path)],text=True))
    subprocess.run(['ffmpeg','-v','error','-xerror','-i',str(path),'-f','null','-'],check=True)
    return probe


def render(directory,speed,fps,preview=False):
    root=Path(directory);manifest=json.loads((root/'prepared.json').read_text())
    if speed<=0 or not math.isfinite(speed) or fps<=0:
        raise ValueError('speed and fps must be positive')
    for cmd in ['ffmpeg','ffprobe','fc-match']:
        if not shutil.which(cmd):raise ValueError(f'required command is missing: {cmd}')
    scenes=[Scene(manifest,i,speed) for i in range(len(manifest['scenes']))]
    if preview:
        cover(manifest).save(root/'intro-preview.png')
        for i,scene in enumerate(scenes):scene.frame(scene.s['duration_s']/3).save(root/f'scene-{i+1:02d}-preview.png')
        cover(manifest,True).save(root/'ending-preview.png');return
    output=root/'replay.partial.mp4';final=root/'replay.mp4'
    if output.exists() or final.exists():raise ValueError('output already exists; use a fresh prepared directory')
    chapters=[('片头',4*fps)];chunks=[]
    for scene in scenes:
        count=max(1,round(scene.s['duration_s']/speed*fps));chunks.append(count)
        chapters.append((scene.s['title'],2*fps+count+round(1.5*fps)))
    chapters.append(('原回执汇总',5*fps));cursor=0;meta=[';FFMETADATA1']
    for title,count in chapters:
        title=title.replace('\\','\\\\').replace('\n',' ').replace('=','\\=').replace(';','\\;').replace('#','\\#')
        meta.extend(['[CHAPTER]','TIMEBASE=1/1000',f'START={round(cursor/fps*1000)}',f'END={round((cursor+count)/fps*1000)}','title='+title]);cursor+=count
    (root/'chapters.ffmeta').write_text('\n'.join(meta)+'\n')
    command=['ffmpeg','-hide_banner','-loglevel','warning','-y','-thread_queue_size','64','-f','rawvideo','-pix_fmt','rgb24','-s',f'{W}x{H}','-r',str(fps),'-i','pipe:0','-i',str(root/'chapters.ffmeta'),'-map_metadata','1','-map_chapters','1','-c:v','libx264','-preset','veryfast','-crf','20','-pix_fmt','yuv420p','-threads','4','-movflags','+faststart',str(output)]
    frames=0
    with (root/'encode.log').open('w') as log:
        proc=subprocess.Popen(command,stdin=subprocess.PIPE,stderr=log)
        def emit(image,count=1):
            nonlocal frames
            raw=image.tobytes()
            for _ in range(count):proc.stdin.write(raw);frames+=1
        try:
            emit(cover(manifest),4*fps)
            for scene,count in zip(scenes,chunks):
                emit(scene.frame(0,True),2*fps)
                for k in range(count):emit(scene.frame(min(k*speed/fps,scene.s['duration_s'])))
                emit(scene.frame(scene.s['duration_s']),round(1.5*fps));print(f'Rendered {scene.s["title"]}',flush=True)
            emit(cover(manifest,True),5*fps)
            proc.stdin.close()
            if proc.wait()!=0:raise RuntimeError('ffmpeg encode failed; partial output and encode.log retained')
        except BaseException:
            if proc.poll() is None:proc.kill()
            proc.wait();raise
    probe=verify_video(output)
    stream=next(s for s in probe['streams'] if s['codec_type']=='video')
    if (stream['width'],stream['height'],int(stream['nb_frames']))!=(W,H,frames) or len(probe['chapters'])!=len(chapters):raise RuntimeError('encoded frame/size/chapter verification failed')
    digest=source(output)['sha256']
    output.replace(final)
    (root/'verification.json').write_text(json.dumps({'passed':True,'video':str(final),'sha256':digest,'width':W,'height':H,'fps':fps,'frames':frames,'duration_s':frames/fps,'speed':speed,'full_decode_passed':True,'is_live_recording':False,'chapters':probe['chapters'],'prepared_manifest_sha256':source(root/'prepared.json')['sha256']},ensure_ascii=False,indent=2)+'\n')
    print(f'Verified {final}: {frames/fps:.2f}s, {frames} frames')


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('directory',type=Path);parser.add_argument('--speed',type=float,default=3);parser.add_argument('--fps',type=int,default=24);parser.add_argument('--preview',action='store_true')
    args=parser.parse_args();render(args.directory,args.speed,args.fps,args.preview)
