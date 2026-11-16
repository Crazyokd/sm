"""Validate recorded two-robot evidence and prepare a replay without running ROS."""
import argparse
from bisect import bisect_right
import csv
import hashlib
import json
import math
from pathlib import Path
import xml.etree.ElementTree as ET

HEADER = 'schema,iteration,sim_ns,dt_ns,model,link_entity,valid,px,py,pz,qx,qy,qz,qw,vx,vy,vz,wx,wy,wz,error'.split(',')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def source(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(chunk)
    return {'path': str(path), 'sha256': digest.hexdigest()}


def number(value):
    require(type(value) in (int, float) and math.isfinite(value), 'expected a finite number')
    return float(value)


def integer(value):
    require(type(value) is int and value >= 0, 'expected a nonnegative integer timestamp')
    return value


def pose(element):
    p = element.find('pose')
    require(p is None or not p.get('relative_to'), 'relative_to frames require an explicit geometry export')
    values = list(map(float, p.text.split())) if p is not None else [0.] * 6
    require(len(values) == 6 and all(math.isfinite(x) for x in values), 'invalid SDF pose')
    require(abs(values[3]) < 1e-9 and abs(values[4]) < 1e-9, 'tilted walls require an explicit geometry export')
    return values[0], values[1], values[5]


def transform(points, placement):
    x, y, a = placement
    c, s = math.cos(a), math.sin(a)
    return [[x + c * px - s * py, y + s * px + c * py] for px, py in points]


def wall_polygons(path, model_name):
    model = next((m for m in ET.parse(path).getroot().findall('world/model') if m.get('name') == model_name), None)
    require(model is not None, f'wall model not found: {model_name}')
    walls = []
    for link in model.findall('link'):
        for collision in link.findall('collision'):
            size = collision.findtext('geometry/box/size')
            require(size is not None, 'wall collisions must be boxes; do not approximate unknown geometry')
            size = list(map(float, size.split()))
            require(len(size) == 3 and all(math.isfinite(x) and x > 0 for x in size), 'invalid wall dimensions')
            x, y = size[0] / 2, size[1] / 2
            points = [[-x, -y], [x, -y], [x, y], [-x, y]]
            for placement in (pose(collision), pose(link), pose(model)):
                points = transform(points, placement)
            walls.append(points)
    require(walls, 'selected model has no collision walls')
    return walls


def physics(path, robot, start, end):
    rows = []
    first = previous = entity = None
    with path.open(newline='') as stream:
        reader = csv.reader(stream)
        require(next(reader, None) == HEADER, 'unsupported native physics CSV header')
        for row in reader:
            require(len(row) == len(HEADER), 'truncated physics row')
            stamp = int(row[2])
            if not start <= stamp <= end:
                continue
            require(row[0] == '1' and row[4] == robot and row[6] == '1' and not row[20], 'invalid native physics record')
            iteration, dt, identity = int(row[1]), int(row[3]), int(row[5])
            require(dt > 0 and identity > 0, 'invalid physics step or entity')
            if previous is not None:
                require(iteration == previous[0] + 1 and stamp - previous[1] == dt and identity == entity, 'missing native update, time gap or entity change')
            else:
                first, entity = stamp, identity
            values = list(map(float, row[7:20]))
            require(all(math.isfinite(v) for v in values), 'nonfinite physics value')
            x, y, z, w = values[3:7]
            require(abs(x*x + y*y + z*z + w*w - 1) < 1e-5, 'invalid pose quaternion')
            yaw = math.atan2(2*(w*z+x*y), 1-2*(y*y+z*z))
            rows.append([(stamp-start)/1e9, values[0], values[1], yaw, math.sqrt(sum(v*v for v in values[7:10])), math.sqrt(sum(v*v for v in values[10:13]))])
            previous = iteration, stamp
    require(len(rows) >= 2 and first == start and previous[1] == end, 'physics source does not cover exact window start/end')
    return rows


def recommendations(path, start, end):
    if path is None:
        return []
    records = []
    with path.open() as stream:
        for line in stream:
            message = json.loads(line)
            if message['kind'] != 'cooperation':
                continue
            stamp = integer(message['sim_ns'])
            if not start <= stamp <= end:
                continue
            require(type(message['state']) is int and message['state'] in (0, 1, 2, 3), 'invalid cooperation model state')
            recs = [c['recommendation'] for c in message['components']]
            require(all(type(r) is int and r in (0, 1, 2, 3) for r in recs), 'invalid recommendation')
            rec = max(recs, default=0) if message['state'] == 1 else None
            records.append([(stamp-start)/1e9, rec])
    require(all(a[0] <= b[0] for a, b in zip(records, records[1:])), 'cooperation clock is not monotonic')
    return records


def advice_at(records, timestamp):
    index = bisect_right([r[0] for r in records], timestamp) - 1
    return (records[index][1], records[index][0]) if index >= 0 else (None, None)


def completion_at(anchor, timestamp):
    return None if anchor is None else timestamp >= anchor


def prepare(config_path):
    config_path = Path(config_path).resolve()
    c = json.loads(config_path.read_text())
    require(set(c) == {'source_label', 'world', 'body', 'scenes'}, 'config requires source_label, world, body, scenes only')
    require(isinstance(c['source_label'], str) and c['source_label'].strip(), 'source_label must be nonempty')
    def path(value):
        require(isinstance(value, str) and value, 'source path must be a nonempty string')
        return (config_path.parent / value).resolve()
    world, model = path(c['world']['sdf']), path(c['body']['source'])
    outline = [[number(x), number(y)] for x, y in c['body']['outline']]
    area = sum(a[0]*b[1]-b[0]*a[1] for a, b in zip(outline, outline[1:] + outline[:1]))
    require(len(outline) >= 3 and abs(area) > 1e-9, 'body outline must enclose the validated collision body')
    manifest = {'source_label': c['source_label'], 'body_polygon': outline, 'walls': wall_polygons(world, c['world']['wall_model']), 'assets': {'world': source(world), 'body': source(model)}, 'scenes': []}
    require(isinstance(c['scenes'], list) and c['scenes'], 'at least one scene is required')
    for scene in c['scenes']:
        require(set(scene) == {'title', 'result', 'physics', 'cooperation', 'goals'}, 'scene requires title, result, physics, cooperation, goals only')
        require(isinstance(scene['title'], str) and scene['title'].strip(), 'scene title must be nonempty')
        require(all(isinstance(scene[k], list) and len(scene[k]) == 2 for k in ['physics', 'cooperation', 'goals']), 'each scene must identify exactly two robots')
        result_path = path(scene['result'])
        result = json.loads(result_path.read_text())
        require(type(result['passed']) is bool, 'result.passed must be boolean; physical success is not overall success')
        start = integer(result['motion_start_anchor']['sim_ns'])
        end = integer(result['rtf_sim_end'])
        require(end > start, 'result window must have a positive duration')
        s = {'title': scene['title'], 'passed': result['passed'], 'failure': result.get('failure'), 'duration_s': (end-start)/1e9, 'sim_start_ns': start, 'sim_end_ns': end, 'physics': result.get('physical'), 'real_time_factor': result.get('real_time_factor'), 'goals': {str(i+1): {'x': number(g['x']), 'y': number(g['y'])} for i, g in enumerate(scene['goals'])}, 'poses': {}, 'advice': {}, 'terminal_s': {}, 'sources': {'result': source(result_path), 'physics': [], 'cooperation': []}}
        for i in (1, 2):
            p = path(scene['physics'][i-1]);co = path(scene['cooperation'][i-1]) if scene['cooperation'][i-1] is not None else None
            s['poses'][str(i)] = physics(p, f'robot{i}', start, end)
            s['advice'][str(i)] = recommendations(co, start, end)
            anchor = result.get('terminal_anchors', {}).get(str(i))
            s['terminal_s'][str(i)] = (integer(anchor['sim_ns'])-start)/1e9 if anchor is not None else None
            if anchor is not None:
                require(0 <= s['terminal_s'][str(i)] <= s['duration_s'], 'terminal anchor is outside the replay window')
            s['sources']['physics'].append(source(p))
            s['sources']['cooperation'].append(source(co) if co is not None else None)
        require([r[0] for r in s['poses']['1']] == [r[0] for r in s['poses']['2']], 'robot physics windows do not share the same steps')
        xy = [r[1:3] for robot in s['poses'].values() for r in robot] + [[g['x'],g['y']] for g in s['goals'].values()]
        s['bounds'] = [min(x for x,y in xy)-1.5, max(x for x,y in xy)+1.5, min(y for x,y in xy)-1.5, max(y for x,y in xy)+1.5]
        manifest['scenes'].append(s)
    return manifest


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('config', type=Path)
    parser.add_argument('output', type=Path, help='new output directory; source recordings remain read-only')
    args = parser.parse_args()
    manifest = prepare(args.config)
    args.output.mkdir(parents=True, exist_ok=False)
    (args.output / 'prepared.json').write_text(json.dumps(manifest, ensure_ascii=False, separators=(',', ':'))+'\n')
    print(f'Prepared {len(manifest["scenes"])} scenes in {args.output / "prepared.json"}')
