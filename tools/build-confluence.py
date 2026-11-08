#!/usr/bin/env python3
"""Build clean Confluence scenario data; extract private scenery from existing exports.

All lengths, colors and placements here are authored design choices, not native rules.
Run from any directory. --assets refreshes a small private GLB from a retail export.
"""
import argparse
import copy
import hashlib
import json
import math
from pathlib import Path
import struct
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[3]
SCENARIOS = ROOT / 'output/rewrite/scenarios'
CACHE = ROOT / 'output/.cache'
GROUND = 'textures/Levels/Textures/Surface/Nature/Ground/'
STONE = 'textures/Levels/Textures/Surface/Structure/Stone/'


def extract_scenery():
    """Keep only the compact ash tree and checkpoint meshes, with original buffers.

    Selection is by decoded material and primitive bounds, not copied retail geometry.
    Full buffers stay private. Paths are rewritten relative to the private output.
    """
    source = CACHE / 'gltf/Levels/Swamp/sw_s1.glb'
    data = source.read_bytes()
    length = struct.unpack_from('<I', data, 12)[0]
    gltf = json.loads(data[20:20 + length])
    binary = data[28 + length:]
    def read_accessor(index):
        a = gltf['accessors'][index]; view = gltf['bufferViews'][a['bufferView']]
        fmt = {5126:'f',5125:'I',5123:'H',5121:'B'}[a['componentType']]
        n = {'SCALAR':1,'VEC2':2,'VEC3':3,'VEC4':4}[a['type']]
        stride = view.get('byteStride',struct.calcsize(fmt)*n)
        offset = view.get('byteOffset',0)+a.get('byteOffset',0)
        return [struct.unpack_from('<'+fmt*n,binary,offset+k*stride) for k in range(a['count'])]
    choices = []
    for primitive in gltf['meshes'][0]['primitives']:
        material = gltf['materials'][primitive['material']]
        acc = gltf['accessors'][primitive['attributes']['POSITION']]
        lo, hi = acc['min'], acc['max']
        if material.get('name') == 'tree_ash00_0' and all(hi[i]-lo[i] < v for i, v in enumerate([8, 16, 8])):
            choices.append(primitive)
    if len(choices) != 1:
        raise ValueError(f'expected one compact ash tree, got {len(choices)}; inspect export')
    out = CACHE / 'gltf/Custom/Confluence'
    out.mkdir(parents=True, exist_ok=True)
    manifest = {'source': str(source.relative_to(ROOT)), 'source_sha256': hashlib.sha256(data).hexdigest(), 'assets': {}}
    spruce = gltf['meshes'][0]['primitives'][43]
    positions = read_accessor(spruce['attributes']['POSITION'])
    indices = [v[0] for v in read_accessor(spruce['indices'])]
    # An isolated spruce in the exported world-space buffer; keep its foliage and trunk.
    spruce_indices = [i for t in zip(*[iter(indices)]*3)
                      if all(-68.2 < positions[i][0] < -62 and 0.8 < positions[i][2] < 7.2 for i in t)
                      for i in t]
    if len(spruce_indices) < 150: raise ValueError('spruce selection no longer matches export')
    for name, primitives in [('ash', choices), ('spruce', [spruce]), ('rock',[gltf['meshes'][0]['primitives'][17]]), ('log',[gltf['meshes'][0]['primitives'][27]]), ('shrine', gltf['meshes'][2]['primitives'])]:
        doc = copy.deepcopy(gltf)
        doc.pop('animations', None)
        doc.pop('skins', None)
        doc['bufferViews'] = []; doc['accessors'] = []
        compact = bytearray(); rebuilt = []; bounds = []
        def pack_accessor(values, typ, component=5126):
            compact.extend(b'\0' * (-len(compact)%4))
            offset=len(compact); fmt={5126:'f',5125:'I'}[component]
            for value in values: compact.extend(struct.pack('<'+fmt*len(value),*value))
            vi=len(doc['bufferViews']);doc['bufferViews'].append({'buffer':0,'byteOffset':offset,'byteLength':len(compact)-offset})
            ai=len(doc['accessors']);a={'bufferView':vi,'componentType':component,'count':len(values),'type':typ}
            if typ=='VEC3':
                a['min']=[min(v[i] for v in values) for i in range(3)]
                a['max']=[max(v[i] for v in values) for i in range(3)]
            doc['accessors'].append(a)
            return ai
        for original in primitives:
            selected = spruce_indices if name=='spruce' else [v[0] for v in read_accessor(original['indices'])]
            used=sorted(set(selected)); remap={old:new for new,old in enumerate(used)}
            primitive={'attributes':{},'material':original['material']}
            for semantic,index in original['attributes'].items():
                # Original deferred materials explicitly disable this zero-filled channel.
                # Standard glTF PBR multiplies alpha by it, hiding the entire tree.
                if semantic=='COLOR_0': continue
                values=read_accessor(index); values=[values[i] for i in used]
                new=pack_accessor(values,gltf['accessors'][index]['type'])
                primitive['attributes'][semantic]=new
                if semantic=='POSITION': bounds.append(doc['accessors'][new])
            primitive['indices']=pack_accessor([(remap[i],) for i in selected],'SCALAR',5125)
            rebuilt.append(primitive)
        compact.extend(b'\0' * (-len(compact)%4))
        doc['buffers']=[{'byteLength':len(compact)}]
        doc['meshes'] = [{'name': name, 'primitives': rebuilt}]
        lo = [min(a['min'][i] for a in bounds) for i in range(3)]
        hi = [max(a['max'][i] for a in bounds) for i in range(3)]
        s = 1 / (hi[1]-lo[1])
        for material in doc['materials']:
            material.pop('extras',None)
            material['doubleSided']=True
        doc['nodes'] = [{'mesh': 0, 'name': name, 'scale': [s]*3,
                         'translation': [-(lo[0]+hi[0])*0.5*s, -lo[1]*s, -(lo[2]+hi[2])*0.5*s]}]
        doc['scenes'] = [{'nodes': [0]}]
        doc['scene'] = 0
        for image in doc.get('images', []):
            if 'uri' in image:
                absolute = (source.parent / image['uri']).resolve()
                import os
                image['uri'] = os.path.relpath(absolute, out)
        encoded = json.dumps(doc, separators=(',', ':')).encode()
        encoded += b' ' * (-len(encoded) % 4)
        packed = (struct.pack('<III', 0x46546c67, 2, 28+len(encoded)+len(compact))
                  + struct.pack('<II', len(encoded), 0x4e4f534a) + encoded
                  + struct.pack('<II', len(compact), 0x004e4942) + compact)
        dest = out / f'{name}.glb'
        dest.write_bytes(packed)
        manifest['assets'][name] = {'path': str(dest.relative_to(ROOT)), 'sha256': hashlib.sha256(packed).hexdigest(), 'source_bounds': [lo, hi]}
    (out / 'provenance.json').write_text(json.dumps(manifest, indent=2)+'\n')
    print(out)


def build():
    arena = ET.Element('Arena', version='1')
    rises=[(-30,12,18,18,4,12),(30,-12,18,18,4,12)]
    def terrain_height(x,z):
        if abs(x-z)*math.sqrt(0.5)<=5:
            return 0 if any(abs((x+z)*0.5-f)<=11 for f in [-62,0,62]) else -4
        return max(h*max(0,1-max(abs(x-cx)-w/2,abs(z-cz)-d/2,0)/apron)
                   for cx,cz,w,d,h,apron in rises)
    def node(tag, **attrs):
        if tag in ['Locator','Scenery','Box','Area']:
            key='position' if tag in ['Locator','Scenery'] else 'center'
            if key in attrs:
                x,y,z=attrs[key];attrs[key]=(x,y+max(0,terrain_height(x,z)),z)
        if tag=='Box':attrs['visible']='false'
        return ET.SubElement(arena, tag, {k: fmt(v) for k, v in attrs.items()})
    def locator(name, x, z):
        node('Locator', name=name, position=(x, 1, z))
    def surface(center, size, texture=None, color=(1, 1, 1), yaw=0, tile=6, edge=0):
        attrs = dict(center=center, size=size, color=color, yaw=yaw, tile=tile, edge=edge)
        if texture:
            attrs['texture'] = texture
        node('Surface', **attrs)
    def strip(a, b, width, y, texture, color=(1, 1, 1)):
        dx, dz = b[0]-a[0], b[1]-a[1]
        surface(((a[0]+b[0])/2, y, (a[1]+b[1])/2),
                (width, math.hypot(dx, dz)+0.2), texture, color, math.atan2(dx, dz), edge=min(1.1,width*0.15))
    def scenery(kind, x, z, height, yaw=0):
        node('Scenery', asset=f'gltf/Custom/Confluence/{kind}.glb',
             position=(x, 0, z), scale=(height,)*3, yaw=yaw)
    node('Floor', center=(0, 0, 0), size=(160, 160), cell=2, clearance=0.6, height=2)
    node('CameraBounds',center=(0,0),size=(148,148))
    node('Area',name='camera_blue_jungle',center=(-32,4,12),size=(46,14,46))
    node('Area',name='camera_red_jungle',center=(32,4,-12),size=(46,14,46))
    node('Area',name='camera_river',center=(0,4,0),size=(140,14,140))
    node('River', width=10,surface=0.3,shallowDepth=0.3,deepDepth=4.3,fordHalfLength=11,fords=(-62,0,62))
    for cx,cz,w,d,h,apron in rises:
        node('Rise',center=(cx,cz),size=(w,d),height=h,apron=apron)
    surface((0, 0.01, 0), (160, 160), GROUND+'grass_lush00_0.png', (1, 1, 1))
    lanes = {
        'top': [(-62, 62), (-62, -62), (62, -62)],
        'mid': [(-62, 62), (0, 0), (62, -62)],
        'bot': [(-62, 62), (62, 62), (62, -62)],
    }
    # Water surface is generated from River by the runtime, including ice collision.
    strip((-75,-75),(75,75),16,0.02,GROUND+'forestfloor00_0.png')
    camps = [(-43, -17), (-17, -43), (-14, 40), (14, -40), (43, 17), (17, 43)]
    objectives = [(-32, -18), (32, 18)]
    jungle_paths = [((-62, -17), (-32, -18)), ((-17, -62), (-32, -18)),
                    ((-32, -18), (-5, 5)), ((-40, 40), (-14, 40)),
                    ((-14, 40), (32, 18)), ((32, 18), (5, -5)),
                    ((62, 17), (32, 18)), ((17, 62), (32, 18)),
                    ((40, -40), (14, -40)), ((14, -40), (-32, -18))]
    for a, b in jungle_paths:
        strip(a, b, 5, 0.035, GROUND+'forestfloor00_0.png')
    for points in lanes.values():
        for a, b in zip(points, points[1:]):
            strip(a, b, 12, 0.05, STONE+'floor_cobblegrey_0.png', (0.85, 0.82, 0.72))
    for i, (x, z) in enumerate(camps):
        surface((x, 0.06, z), (10, 10), GROUND+'forestfloor00_0.png')
        locator(f'camp_{i}', x, z)
        node('Area', name=f'camp_{i}', center=(x, 1, z), size=(10, 4, 10))
        for dx, dz in [(-4,-4), (4,-4), (-4,4), (4,4)]:
            surface((x+dx, 0.07, z+dz), (0.5, 0.5), color=(0.8, 0.62, 0.2))
    for i, (x, z) in enumerate(objectives):
        locator(f'objective_{i}', x, z)
        node('Area', name=f'objective_{i}', center=(x, 1, z), size=(14, 5, 14))
        surface((x, 0.06, z), (14, 14), STONE+'floor_cobblegrey_0.png', (0.55, 0.7, 0.73))
        scenery('shrine', x, z, 5)
        node('Box', center=(x, 1.5, z), size=(2, 3, 2))
    for cx,cz,_,_,_,_ in rises:
        for dx,dz,w,d in [(-15,0,5,20),(15,0,5,20),(0,-15,20,5),(0,15,20,5)]:
            surface((cx+dx,0.03,cz+dz),(w,d),'textures/Levels/Textures/Surface/Nature/Rock/cliff03_0.png',edge=0.5)
    locator('highground_blue',-30,12)
    locator('highground_red',30,-12)
    locator('river_shallow',0,0)
    locator('river_deep',-40,-40)
    for team, (bx, bz), color in [('blue', (-62, 62), (0.18, 0.45, 0.7)), ('red', (62, -62), (0.65, 0.22, 0.15))]:
        sign = 1 if team == 'blue' else -1
        surface((bx, 0.065, bz), (24, 24), STONE+'floor_cobblegrey_0.png', (0.8, 0.8, 0.8))
        surface((bx, 0.075, bz), (9, 9), STONE+'floor_cobblegrey_0.png', color=tuple(0.65+v*0.35 for v in color))
        scenery('shrine', bx, bz, 6)
        node('Box', center=(bx, 2, bz), size=(3, 4, 3))
        locator(f'{team}_core', bx, bz)
        node('Area', name=f'{team}_base', center=(bx, 1, bz), size=(24, 5, 24))
        for i, (dx, dz) in enumerate([(5,-5),(8,-5),(5,-8),(8,-8)]):
            locator(f'{team}_spawn{i}', bx+dx*sign, bz+dz*sign)
            if team == 'blue': locator(f'start{i}', bx+dx, bz+dz)
    # Ordered lane waypoints and off-center tower sites on each half.
    tower_sites = {'top': [(-66, 32), (-66, -32), (-32, -66), (32, -66)],
                   'mid': [(-43, 37), (-22, 16), (22, -16), (43, -37)],
                   'bot': [(-32, 66), (32, 66), (66, 32), (66, -32)]}
    for name, points in lanes.items():
        for i, (x,z) in enumerate(points): locator(f'lane_{name}_{i}', x,z)
        for i, (x,z) in enumerate(tower_sites[name]):
            locator(f'tower_{name}_{i}', x,z)
            scenery('shrine', x,z,4)
            node('Box', center=(x,1.5,z), size=(2,3,2))
    paths = [(a,b) for points in lanes.values() for a,b in zip(points,points[1:])]
    def distance(p, a, b):
        dx,dz = b[0]-a[0], b[1]-a[1]
        t = max(0, min(1, ((p[0]-a[0])*dx+(p[1]-a[1])*dz)/(dx*dx+dz*dz)))
        return math.hypot(p[0]-a[0]-t*dx, p[1]-a[1]-t*dz)
    # Symmetric forest islands, with paths/camp exits carved before blocker placement.
    for x in range(-72, 73, 6):
        grid_x=x
        for z in range(-72, 73, 6):
            grid_z=z
            x=grid_x+0.6*math.sin(grid_x*0.31+grid_z*0.12)
            z=grid_z+0.6*math.sin(grid_z*0.31+grid_x*0.12)
            if abs(x-z)<11 or any(distance((x,z),a,b)<9 for a,b in paths): continue
            if any(distance((x,z),a,b)<4.8 for a,b in jungle_paths):continue
            if any(math.hypot(x-cx,z-cz)<11 for cx,cz in camps+objectives+[(-62,62),(62,-62),(-30,12),(30,-12)]): continue
            height = 7.5 + ((grid_x*grid_x+grid_z*grid_z)//36 % 4)*0.6
            scenery('spruce' if (x+z)//6%4 else 'ash', x,z,height, ((x*17+z*13)%360)*math.pi/180)
            node('Box', center=(x,0.8,z), size=(2.8,1.6,2.8))
            if (grid_x*grid_x+grid_z*grid_z)//36%3==0:
                scenery('rock',x+1.4,z+0.5,0.9,(x+z)*0.3)
            if (x+z)//6%7==0:
                scenery('log',x,z,0.5,x*0.2)
    # Solid perimeter prevents walking off the authored floor.
    for x,z,w,d in [(0,-80,162,2),(0,80,162,2),(-80,0,2,160),(80,0,2,160)]:
        node('Box', center=(x,2,z), size=(w,4,d))
    node('DirectionalLight', name='confluence_sun', direction=(0.3,-1,0.4),
         diffuse=(0.95,0.88,0.74), ambient=(0.36,0.43,0.48))
    ET.indent(arena)
    (SCENARIOS/'confluence.arena').write_text('<!-- Authored MOBA experiment; all placements are design choices. -->\n'+ET.tostring(arena,encoding='unicode')+'\n')
    (SCENARIOS/'confluence.lvl').write_text('<Level>\n  <Start scene="confluence" area="start"/>\n  <Scenes><Scene>confluence.xml</Scene></Scenes>\n</Level>\n')
    (SCENARIOS/'confluence.xml').write_text('''<!-- Layout only; no scenario scripts. Named markers reserve future mod sites. -->
<Scene>
  <Model>confluence.arena</Model>
</Scene>
''')
    print(SCENARIOS/'confluence.lvl')


def fmt(value):
    if isinstance(value, (list,tuple)):
        return ','.join(fmt(v) for v in value)
    if isinstance(value,float):
        return f'{value:.6g}'
    return str(value)


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--assets', action='store_true')
    parser.add_argument('--assets-only', action='store_true',help='Refresh private scenery without rewriting scenario data')
    args=parser.parse_args()
    if not args.assets_only: build()
    if args.assets or args.assets_only: extract_scenery()
