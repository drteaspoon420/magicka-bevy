"""Import the playable Confluence source into a new Blender scene.

Execute via Blender MCP with runpy.run_path(...), then call build(project_root).
Does not clear or replace an existing scene. Derived .blend/render stay private.
The .arena remains authoritative; editing the Blender preview does not update it.
"""
from pathlib import Path
import math
import xml.etree.ElementTree as ET


def build(project_root):
    import bpy
    from mathutils import Vector
    root = Path(project_root)
    cache = root / 'output/.cache'
    source = root / 'output/rewrite/scenarios/confluence.arena'
    arena = ET.parse(source).getroot()
    scene = bpy.data.scenes.new('Confluence — three lane sandbox')
    bpy.context.window.scene = scene
    scene.unit_settings.system = 'METRIC'
    scene.world = bpy.data.worlds.new('Confluence sky')
    scene.world.use_nodes = True
    scene.world.node_tree.nodes['Background'].inputs[0].default_value = (0.14,0.19,0.25,1)
    scene.world.node_tree.nodes['Background'].inputs[1].default_value = 0.5
    groups = {}
    for name in ['Terrain', 'Scenery', 'Gameplay markers', 'Collision', 'Overview']:
        group = bpy.data.collections.new('Confluence / '+name)
        scene.collection.children.link(group)
        groups[name] = group
    groups['Collision'].hide_render = True
    def vec(n,key): return tuple(float(v) for v in n.attrib[key].split(','))
    def coord(v): return (v[0],-v[2],v[1])
    def obj(name, data, group):
        item=bpy.data.objects.new(name,data)
        groups[group].objects.link(item)
        return item
    river=arena.find('River')
    def terrain_height(x,z):
        if river is not None and abs(x-z)*math.sqrt(0.5)<=float(river.get('width'))/2:
            along=(x+z)*0.5
            ford=any(abs(along-float(v))<=float(river.get('fordHalfLength')) for v in river.get('fords').split(','))
            return float(river.get('surface'))-float(river.get('shallowDepth' if ford else 'deepDepth'))
        heights=[0.]
        for n in arena.findall('Rise'):
            cx,cz=vec(n,'center');w,d=vec(n,'size')
            distance=max(abs(x-cx)-w/2,abs(z-cz)-d/2,0)
            heights.append(float(n.get('height'))*max(0,1-distance/float(n.get('apron'))))
        return max(heights)
    def ground(x,z):
        # Same 2 m cell triangles as the runtime's collision mesh.
        xa=math.floor(x/2)*2;za=math.floor(z/2)*2;u=(x-xa)/2;v=(z-za)/2
        a=terrain_height(xa,za);b=terrain_height(xa,za+2)
        c=terrain_height(xa+2,za+2);d=terrain_height(xa+2,za)
        return a*(1-v)+b*(v-u)+c*u if v>=u else a*(1-u)+c*v+d*(u-v)
    materials={}
    for i,n in enumerate(arena.findall('Surface')):
        w,d=vec(n,'size'); tint=vec(n,'color'); tile=float(n.attrib['tile'])
        center=vec(n,'center');yaw=float(n.attrib['yaw']);cs=math.cos(yaw);sn=math.sin(yaw)
        nx=math.ceil(w/2);nz=math.ceil(d/2);verts=[];uvs=[];faces=[]
        for z in range(nz+1):
            for x in range(nx+1):
                lx=-w/2+w*x/nx;lz=-d/2+d*z/nz
                wave=math.sin(lz*0.21)*0.65+math.sin(lz*0.49)*0.35
                lx*=1+float(n.get("edge","0"))*wave*2/w
                px=center[0]+cs*lx+sn*lz;pz=center[2]-sn*lx+cs*lz
                verts.append((px,-pz,ground(px,pz)+center[1]));uvs.append(((lx+w/2)/tile,(lz+d/2)/tile))
        for z in range(nz):
            for x in range(nx):
                a=z*(nx+1)+x;b=a+nx+1;c=b+1;d1=a+1
                faces.extend([(a,b,c),(a,c,d1)])
        mesh=bpy.data.meshes.new('Surface');mesh.from_pydata(verts,[],faces)
        uv=mesh.uv_layers.new()
        for polygon in mesh.polygons:
            for li in polygon.loop_indices:uv.data[li].uv=uvs[mesh.loops[li].vertex_index]
        item=obj(f'Surface {i:03}',mesh,'Terrain')
        texture=n.get('texture')
        key=(texture,tint)
        if key not in materials:
            mat=bpy.data.materials.new(Path(texture).stem if texture else 'Authored accent')
            mat.diffuse_color=(*tint,1);mat.use_nodes=True
            nodes=mat.node_tree.nodes;bsdf=nodes.get('Principled BSDF')
            bsdf.inputs['Base Color'].default_value=(*tint,1);bsdf.inputs['Roughness'].default_value=0.95
            if texture:
                tex=nodes.new('ShaderNodeTexImage');tex.image=bpy.data.images.load(str(cache/texture),check_existing=True)
                mix=nodes.new('ShaderNodeMixRGB');mix.blend_type='MULTIPLY';mix.inputs[0].default_value=1
                mix.inputs[2].default_value=(*tint,1)
                mat.node_tree.links.new(tex.outputs['Color'],mix.inputs[1])
                mat.node_tree.links.new(mix.outputs[0],bsdf.inputs['Base Color'])
            materials[key]=mat
        mesh.materials.append(materials[key])
    if river is not None:
        materials_water=[]
        for name,color in [('Shallow ford',(0.18,0.38,0.32,1)),('Deep channel',(0.025,0.11,0.14,1))]:
            mat=bpy.data.materials.new(name);mat.use_nodes=True
            bsdf=mat.node_tree.nodes.get('Principled BSDF');bsdf.inputs['Base Color'].default_value=color
            bsdf.inputs['Roughness'].default_value=0.2;bsdf.inputs['Metallic'].default_value=0.12
            mat.diffuse_color=color;materials_water.append(mat)
        verts=[];faces=[];deep=[];surface=float(river.get('surface'))
        across=math.ceil(float(river.get('width'))/2);along=math.ceil(160*math.sqrt(2)/2)
        width=float(river.get('width'))
        for z in range(along+1):
            for x in range(across+1):
                at=-80+160*z/along;side=(-width/2+width*x/across)*math.sqrt(0.5)
                px=max(-80,min(80,at+side));pz=max(-80,min(80,at-side))
                verts.append((px,-pz,surface))
        for z in range(along):
            for x in range(across):
                a=z*(across+1)+x;b=a+across+1;c=b+1;d1=a+1
                for tri in [(a,b,c),(a,c,d1)]:
                    faces.append(tri);cx=sum(verts[i][0] for i in tri)/3;cz=-sum(verts[i][1] for i in tri)/3
                    deep.append(1 if terrain_height(cx,cz)<-1 else 0)
        mesh=bpy.data.meshes.new('Water and ice surface');mesh.from_pydata(verts,[],faces)
        for mat in materials_water:mesh.materials.append(mat)
        for polygon,index in zip(mesh.polygons,deep):polygon.material_index=index
        obj('River / shallow fords + deep channel',mesh,'Terrain')
    libraries={}
    for n in arena.findall('Scenery'):
        asset=n.attrib['asset']
        if asset not in libraries:
            before=set(bpy.data.objects)
            bpy.ops.import_scene.gltf(filepath=str(cache/asset))
            imported=set(bpy.data.objects)-before
            library=bpy.data.collections.new('Confluence library / '+Path(asset).stem)
            for item in imported:
                for group in list(item.users_collection): group.objects.unlink(item)
                library.objects.link(item)
            libraries[asset]=library
        item=obj(Path(asset).stem,None,'Scenery')
        item.instance_type='COLLECTION';item.instance_collection=libraries[asset]
        item.location=coord(vec(n,'position'));sx,sy,sz=vec(n,'scale');item.scale=(sx,sz,sy)
        item.rotation_euler[2]=float(n.attrib['yaw'])
    for n in arena.findall('Locator'):
        item=obj(n.attrib['name'],None,'Gameplay markers')
        item.location=coord(vec(n,'position'));item.empty_display_type='ARROWS';item.empty_display_size=1.2
        item['magicka_locator']=n.attrib['name']
    for n in arena.findall('Area'):
        item=obj(n.attrib['name']+' trigger',None,'Gameplay markers')
        item.location=coord(vec(n,'center'));x,y,z=vec(n,'size');item.scale=(x/2,z/2,y/2)
        item.empty_display_type='CUBE';item['magicka_area']=n.attrib['name']
    for i,n in enumerate(arena.findall('Box')):
        # Editable wire markers for authoritative collision in the runtime.
        item=obj(f'Collision {i:03}',None,'Collision')
        item.location=coord(vec(n,'center'));x,y,z=vec(n,'size');item.scale=(x/2,z/2,y/2)
        item.empty_display_type='CUBE';item['magicka_collision']=True
    bounds=arena.find('CameraBounds')
    if bounds is not None:
        item=obj('Camera bounds',None,'Gameplay markers');cx,cz=vec(bounds,'center');w,d=vec(bounds,'size')
        item.location=(cx,-cz,0);item.scale=(w/2,d/2,0.1);item.empty_display_type='CUBE'
        item['magicka_camera_bounds']=True
    light=bpy.data.lights.new('Confluence sun','SUN');light.energy=3.5;light.angle=0.2
    sun=obj('Confluence sun',light,'Overview');sun.rotation_euler=(0.35,-0.45,-0.3)
    camera=bpy.data.cameras.new('Confluence overview');camera.type='ORTHO';camera.ortho_scale=205
    cam=obj('Confluence overview',camera,'Overview');cam.location=(0,-100,245)
    cam.rotation_euler=(Vector((0,0,0))-cam.location).to_track_quat('-Z','Y').to_euler();scene.camera=cam
    # Named gameplay sites remain discoverable in the Outliner, including inactive objectives.
    scene.render.engine='CYCLES';scene.cycles.samples=12
    scene.render.resolution_x=1600;scene.render.resolution_y=1600;scene.render.resolution_percentage=100
    scene.view_settings.view_transform='AgX';scene.view_settings.look='AgX - Medium High Contrast';scene.view_settings.exposure=1.0
    for area in bpy.context.screen.areas:
        if area.type=='VIEW_3D':
            area.spaces.active.shading.type="MATERIAL"
            area.spaces.active.region_3d.view_rotation=cam.rotation_euler.to_quaternion()
            area.spaces.active.region_3d.view_perspective="ORTHO"
            area.spaces.active.region_3d.view_distance=210
            area.spaces.active.region_3d.view_location=(0,0,0)
            area.spaces.active.clip_end=2000
    out=cache/'confluence';out.mkdir(parents=True,exist_ok=True)
    # Save a copy: keep the user's current file path and other scenes intact.
    scene.render.filepath=str(out/'overview.png')
    bpy.ops.wm.save_as_mainfile(filepath=str(out/'confluence.blend'),copy=True)
    return {'scene':scene.name,'objects':len(scene.objects),'blend':str(out/'confluence.blend'),
            'render':scene.render.filepath,'source':str(source)}
