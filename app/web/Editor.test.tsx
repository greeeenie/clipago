// @vitest-environment jsdom
import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {cleanup,fireEvent,render,screen} from '@testing-library/react';
import {Editor} from './Editor';
import type {Clip} from './api';
vi.mock('@tauri-apps/api/event',()=>({listen:vi.fn().mockResolvedValue(()=>{})}));
vi.mock('@tauri-apps/api/core',()=>({convertFileSrc:(path:string)=>path,invoke:vi.fn()}));
const clip:Clip={path:'clip.mp4',duration:10,width:1920,height:1080,bytes:1_000_000,has_audio:false,audio_tracks:[],audio_mix_index:null,source:null,fps:30,aspect_ratio:16/9,original_path:null,viewed:false,is_movie:false};
beforeEach(()=>{
  vi.stubGlobal('ResizeObserver',class{observe(){}unobserve(){}disconnect(){}});
  vi.stubGlobal('IntersectionObserver',class{observe(){}disconnect(){}});
});
afterEach(()=>{cleanup();vi.restoreAllMocks();vi.unstubAllGlobals();});
it('Enter saves even while a trim handle has keyboard focus',()=>{
  const save=vi.fn();render(<Editor clip={clip} selection={{start:1,end:8,cursor:1}} onChange={()=>{}} compact onSave={save}/>);
  const handle=screen.getByRole('slider',{name:'Начало фрагмента'});handle.focus();fireEvent.keyDown(handle,{key:'Enter',code:'Enter'});expect(save).toHaveBeenCalledTimes(1);
});
it('Space plays from a focused trim handle without triggering save',()=>{
  const play=vi.spyOn(HTMLMediaElement.prototype,'play').mockResolvedValue();const save=vi.fn();
  render(<Editor clip={clip} selection={{start:0,end:10,cursor:0}} onChange={()=>{}} compact onSave={save}/>);
  const handle=screen.getByRole('slider',{name:'Конец фрагмента'});handle.focus();fireEvent.keyDown(handle,{key:' ',code:'Space'});expect(play).toHaveBeenCalledTimes(1);expect(save).not.toHaveBeenCalled();
});
