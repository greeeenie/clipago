// @vitest-environment jsdom
import {useState} from 'react';
import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {act,cleanup,fireEvent,render,screen,waitFor} from '@testing-library/react';
import {Tooltip} from 'radix-ui';
import {Editor} from './Editor';
import {ExportPanel} from './ExportPanel';
import {api,defaultConfig,type Clip,type Snapshot} from './api';
vi.mock('@tauri-apps/api/event',()=>({listen:vi.fn().mockResolvedValue(()=>{})}));
vi.mock('@tauri-apps/api/core',()=>({convertFileSrc:(path:string)=>path,invoke:vi.fn()}));
const clip:Clip={path:'apps.mp4',duration:45,width:1920,height:1080,bytes:20_000_000,has_audio:true,audio_tracks:[{index:1,name:'Deadlock'},{index:2,name:'Discord'}],audio_mix_index:0,source:null,fps:60,aspect_ratio:16/9,original_path:null,viewed:false,is_movie:false};
let audio:HTMLAudioElement[]=[];
const states=new WeakMap<HTMLMediaElement,boolean>();
beforeEach(()=>{
 localStorage.clear();audio=[];
 vi.stubGlobal('ResizeObserver',class{observe(){}unobserve(){}disconnect(){}});vi.stubGlobal('IntersectionObserver',class{observe(){}disconnect(){}});
 vi.stubGlobal('Audio',vi.fn(function(src:string){const el=document.createElement('audio');el.src=src;Object.defineProperty(el,'readyState',{value:4});audio.push(el);return el;}));
 vi.spyOn(HTMLMediaElement.prototype,'paused','get').mockImplementation(function(this:HTMLMediaElement){return states.get(this)??true;});
 vi.spyOn(HTMLMediaElement.prototype,'play').mockImplementation(function(this:HTMLMediaElement){states.set(this,false);this.dispatchEvent(new Event('playing'));return Promise.resolve();});
 vi.spyOn(HTMLMediaElement.prototype,'pause').mockImplementation(function(this:HTMLMediaElement){if(states.get(this)===false){states.set(this,true);this.dispatchEvent(new Event('pause'));}});
 vi.spyOn(HTMLMediaElement.prototype,'load').mockImplementation(()=>{});
 vi.spyOn(api,'previewAudio').mockResolvedValue([{index:1,path:'game.m4a'},{index:2,path:'discord.m4a'}]);
});
afterEach(()=>{cleanup();vi.restoreAllMocks();vi.unstubAllGlobals();});
function Harness(){const [config,setConfig]=useState(defaultConfig(clip,false));return <Tooltip.Provider><Editor clip={clip} audioTracks={config.audioTracks} selection={{start:0,end:45,cursor:0}} onChange={()=>{}}/><ExportPanel clip={clip} config={config} setConfig={setConfig} start={0} end={45} onClose={()=>{}} onExport={()=>{}} busy={false} state={{encoders:['Cpu']} as Snapshot}/></Tooltip.Provider>;}
it('switches mute preview stems immediately without seeking or restarting the remaining sound',async()=>{
 const {container}=render(<Harness/>);await waitFor(()=>expect(audio).toHaveLength(2));
 const video=container.querySelector('video')!;video.currentTime=12;
 await act(async()=>{await video.play();});
 expect(video.muted).toBe(true);expect(audio[0].paused).toBe(false);expect(audio[1].paused).toBe(false);
 expect(audio[0].currentTime).toBe(12);
 audio[0].currentTime=12.03;
 fireEvent.click(screen.getByRole('switch',{name:'Discord'}));
 expect(audio[1].muted).toBe(true);expect(audio[0].muted).toBe(false);expect(audio[0].currentTime).toBe(12.03);expect(video.paused).toBe(false);
 fireEvent.click(screen.getByRole('switch',{name:'Discord'}));expect(audio[1].muted).toBe(false);
 fireEvent.click(screen.getByRole('switch',{name:'Discord'}));fireEvent.click(screen.getByRole('switch',{name:'Deadlock'}));expect(audio.every(a=>a.muted)).toBe(true);expect(video.muted).toBe(true);
});
it('preview volume affects every stem, survives mute, and does not change export settings',async()=>{
 const {container}=render(<Harness/>);await waitFor(()=>expect(audio).toHaveLength(2));
 const video=container.querySelector('video')!;video.currentTime=8;audio[0].currentTime=8.02;
 const slider=screen.getByRole('slider',{name:'Громкость предпросмотра'});
 fireEvent.keyDown(slider,{key:'Home'});expect(audio[0].volume).toBe(0);expect(video.volume).toBe(0);expect(audio[0].currentTime).toBe(8.02);
 fireEvent.keyDown(slider,{key:'ArrowRight'});expect(audio[0].volume).toBe(.01);expect(audio[1].volume).toBe(.01);
 fireEvent.click(screen.getByRole('button',{name:'Выключить звук'}));expect(audio.every(a=>a.muted)).toBe(true);
 fireEvent.click(screen.getByRole('button',{name:'Включить звук'}));expect(audio[0].volume).toBe(.01);expect(audio[0].muted).toBe(false);
 expect(localStorage.getItem('rewind.preview.volume')).toBe('0.01');
 expect(screen.getByRole('switch',{name:'Discord'}).getAttribute('data-state')).toBe('checked');
});
it('keeps sound synchronized on seeking, pausing and hiding the editor',async()=>{
 const {container,rerender}=render(<Tooltip.Provider><Editor clip={clip} audioTracks={[1]} selection={{start:0,end:45,cursor:0}} onChange={()=>{}}/></Tooltip.Provider>);
 await waitFor(()=>expect(audio).toHaveLength(2));const video=container.querySelector('video')!;
 await act(async()=>{await video.play();});video.currentTime=23;fireEvent.seeking(video);expect(audio[0].paused).toBe(true);fireEvent.seeked(video);expect(audio[0].currentTime).toBe(23);expect(audio[0].paused).toBe(false);
 rerender(<Tooltip.Provider><Editor clip={clip} audioTracks={[1]} selection={{start:0,end:45,cursor:23}} onChange={()=>{}} suspended/></Tooltip.Provider>);
 expect(video.paused).toBe(true);expect(audio.every(a=>a.paused)).toBe(true);
});
it('never leaks the original mix while application audio is preparing',async()=>{
 let resolve!:(v:{index:number;path:string}[])=>void;vi.mocked(api.previewAudio).mockReturnValue(new Promise(done=>{resolve=done;}));
 const {container,rerender}=render(<Tooltip.Provider><Editor clip={clip} audioTracks={[1,2]} selection={{start:0,end:45,cursor:0}} onChange={()=>{}}/></Tooltip.Provider>);
 const video=container.querySelector('video')!;await act(async()=>{await video.play();});
 rerender(<Tooltip.Provider><Editor clip={clip} audioTracks={[1]} selection={{start:0,end:45,cursor:0}} onChange={()=>{}}/></Tooltip.Provider>);
 expect(video.muted).toBe(true);expect(video.paused).toBe(true);
 await act(async()=>resolve([{index:1,path:'game.m4a'},{index:2,path:'discord.m4a'}]));
 expect(video.paused).toBe(false);expect(audio[1].muted).toBe(true);expect(video.muted).toBe(true);
});
it('does not resume pending playback behind settings or leave stems running after unmount',async()=>{
 let resolve!:(v:{index:number;path:string}[])=>void;vi.mocked(api.previewAudio).mockReturnValue(new Promise(done=>{resolve=done;}));
 const props={clip,audioTracks:[1],selection:{start:0,end:45,cursor:0},onChange:()=>{}};
 const {container,rerender,unmount}=render(<Tooltip.Provider><Editor {...props} audioTracks={[1,2]}/></Tooltip.Provider>);
 const video=container.querySelector('video')!;
 await act(async()=>{await video.play();});
 rerender(<Tooltip.Provider><Editor {...props}/></Tooltip.Provider>);
 expect(video.paused).toBe(true);
 rerender(<Tooltip.Provider><Editor {...props} suspended/></Tooltip.Provider>);
 await act(async()=>resolve([{index:1,path:'game.m4a'},{index:2,path:'discord.m4a'}]));
 expect(video.paused).toBe(true);expect(audio.every(a=>a.paused)).toBe(true);
 unmount();expect(audio.every(a=>a.paused&&!a.hasAttribute('src'))).toBe(true);
});
it('mutes a legacy mixed track directly without preparing stems',()=>{
 const legacy={...clip,audio_tracks:[{index:0,name:'Системный звук'}],audio_mix_index:null};
 const props={clip:legacy,selection:{start:0,end:45,cursor:0},onChange:()=>{}};
 const {container,rerender}=render(<Tooltip.Provider><Editor {...props} audioTracks={[0]}/></Tooltip.Provider>);
 const video=container.querySelector('video')!;expect(video.muted).toBe(false);
 rerender(<Tooltip.Provider><Editor {...props} audioTracks={[]}/></Tooltip.Provider>);expect(video.muted).toBe(true);
 rerender(<Tooltip.Provider><Editor {...props} audioTracks={[0]}/></Tooltip.Provider>);expect(video.muted).toBe(false);
 expect(api.previewAudio).not.toHaveBeenCalled();
});
it('does not restart a shorter audio stem after it ends',async()=>{
 const {container}=render(<Harness/>);await waitFor(()=>expect(audio).toHaveLength(2));
 const video=container.querySelector('video')!;Object.defineProperty(audio[1],'duration',{value:5});video.currentTime=12;
 await act(async()=>{await video.play();});expect(audio[0].paused).toBe(false);expect(audio[1].paused).toBe(true);
 video.currentTime=2;fireEvent.seeked(video);expect(audio[1].paused).toBe(false);expect(audio[1].currentTime).toBe(2);
});
