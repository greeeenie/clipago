// @vitest-environment jsdom
import {useState} from 'react';
import {beforeEach,afterEach,expect,it,vi} from 'vitest';
import {cleanup,render,screen,fireEvent,waitFor} from '@testing-library/react';
import {Tooltip} from 'radix-ui';
import {Library} from './Library';
import {MovieWorkspace} from './MovieWorkspace';
import {clipGroups,moveSegment,loadProject} from './projectState';
import {api,defaultConfig,recommendedBitrate,effectiveConfig,estimatedSize,type Clip,type MovieProject,type MovieSegment,type Snapshot} from './api';
vi.mock('@tauri-apps/api/event',()=>({listen:vi.fn().mockResolvedValue(()=>{})}));
vi.mock('@tauri-apps/api/core',()=>({invoke:vi.fn(),convertFileSrc:(p:string)=>p}));
const raw:Clip={path:'raw.mp4',duration:45,width:1920,height:1080,bytes:50_000_000,has_audio:false,audio_tracks:[],audio_mix_index:null,source:'Deadlock',fps:60,aspect_ratio:16/9,original_path:null,viewed:false,is_movie:false};
const edited:Clip={...raw,path:'edited.mp4',duration:10,original_path:raw.path,viewed:true};
const state={encoders:['Cpu']} as Snapshot;
beforeEach(()=>{localStorage.clear();vi.stubGlobal('ResizeObserver',class{observe(){}unobserve(){}disconnect(){}});vi.stubGlobal('IntersectionObserver',class{observe(){}disconnect(){}});vi.spyOn(HTMLMediaElement.prototype,'pause').mockImplementation(()=>{});vi.spyOn(HTMLMediaElement.prototype,'load').mockImplementation(()=>{});});
afterEach(()=>{cleanup();vi.restoreAllMocks();vi.unstubAllGlobals();});
it('chooses bitrate from geometry, FPS and codec, independently of source file size',()=>{
 const c=defaultConfig(raw,false);expect(c.automatic).toBe(true);expect(c.bitrate).toBeCloseTo(9.1);
 expect(defaultConfig({...raw,bytes:500_000_000},false).bitrate).toBe(c.bitrate);
 expect(recommendedBitrate(raw,{...c,height:720})).toBeLessThan(c.bitrate);
 expect(recommendedBitrate(raw,{...c,encoding:{...c.encoding,fps:30}})).toBeLessThan(c.bitrate);
 expect(recommendedBitrate(raw,{...c,encoding:{...c.encoding,codec:'Hevc'}})).toBeLessThan(c.bitrate);
 expect(effectiveConfig(raw,{...c,automatic:false,bitrate:2}).bitrate).toBe(2);
 expect(estimatedSize(raw,c,45)).toBeCloseTo(9.1*45/8*1.02);
});
it('groups processed versions under their untouched original and shows unread status',()=>{
 const select=vi.fn();render(<Tooltip.Provider><Library clips={[edited,raw,{...raw,path:'movie.mp4',is_movie:true}]} selected={raw.path} onSelect={select} onImport={()=>{}} onSettings={()=>{}} onError={()=>{}}/></Tooltip.Provider>);
 expect(clipGroups([edited,raw])).toHaveLength(1);expect(screen.getAllByText('Исходник')).toHaveLength(1);expect(screen.getByLabelText('Не просмотрен')).toBeTruthy();
 fireEvent.click(screen.getByText('Обработан'));expect(select).toHaveBeenCalledWith(edited);
 expect(screen.queryByTitle('movie.mp4')).toBeNull();
});
it('reorders references without mutating clips or stored edits and restores the draft',()=>{
 const a:MovieSegment={id:'a',path:raw.path,start:3,end:8,config:defaultConfig(raw,false)};const b={...a,id:'b',path:edited.path};const parts=[a,b];const next=moveSegment(parts,'a',1);
 expect(next.map(p=>p.id)).toEqual(['b','a']);expect(parts.map(p=>p.id)).toEqual(['a','b']);expect(raw.duration).toBe(45);
 localStorage.setItem('rewind.movie.v1',JSON.stringify({name:'Test',segments:next}));expect(loadProject().segments[1].start).toBe(3);
 expect(loadProject('rewind.movie.v1:other-profile').segments).toEqual([]);
});
it('accepts dragged raw clips, duplicates references, and exports the nondestructive sequence',async()=>{
 const changed=vi.fn();const movie=vi.spyOn(api,'movie').mockResolvedValue({...raw,is_movie:true,path:'movie.mp4'});
 function Harness(){const [project,set]=useState<MovieProject>({name:'Мувик',segments:[]});return <Tooltip.Provider><MovieWorkspace clips={[raw,edited]} state={state} project={project} setProject={p=>{changed(p);set(p);}} onImport={()=>{}} onViewed={()=>{}} onSettings={()=>{}} onError={()=>{}} onSaved={()=>{}} onBusy={()=>{}} suspended={false}/></Tooltip.Provider>;}
 render(<Harness/>);const transfer={getData:(type:string)=>type==='application/x-rewind-clip'?edited.path:'',types:['application/x-rewind-clip']};fireEvent.drop(screen.getByLabelText('Монтажная последовательность'),{dataTransfer:transfer});
 expect(changed.mock.calls.at(-1)![0].segments[0].path).toBe(edited.path);
 fireEvent.click(screen.getByRole('button',{name:'Дублировать фрагмент 1'}));expect(changed.mock.calls.at(-1)![0].segments).toHaveLength(2);
 fireEvent.click(screen.getByRole('button',{name:'Экспорт видео'}));
 fireEvent.click(screen.getByRole('button',{name:'Экспортировать'}));
 await waitFor(()=>expect(movie).toHaveBeenCalled());expect(movie.mock.calls[0][0]).toHaveLength(2);expect(movie.mock.calls[0][0][0].start).toBe(0);expect(edited.duration).toBe(10);expect(raw.duration).toBe(45);
});
