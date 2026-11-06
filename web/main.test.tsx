// @vitest-environment jsdom
import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {act,cleanup,fireEvent,render,screen} from '@testing-library/react';
import type {Snapshot} from './api';
const mocks=vi.hoisted(()=>({handlers:new Map<string,(event:{payload:unknown})=>void>(),invoke:vi.fn()}));
vi.mock('@tauri-apps/api/core',()=>({invoke:mocks.invoke,convertFileSrc:(path:string)=>path}));
vi.mock('@tauri-apps/api/event',()=>({listen:vi.fn((name:string,handler:(event:{payload:unknown})=>void)=>{mocks.handlers.set(name,handler);return Promise.resolve(()=>mocks.handlers.delete(name));})}));
vi.mock('@tauri-apps/api/window',()=>({getCurrentWindow:()=>({onDragDropEvent:vi.fn().mockResolvedValue(()=>{}),close:vi.fn(),minimize:vi.fn(),toggleMaximize:vi.fn()})}));
import {App} from './main';
const state:Snapshot={settings:{buffer_seconds:60,fps:30,monitor:0,encoder:'Auto',output_dir:'clips',ffmpeg:'ffmpeg',ffprobe:'ffprobe',shortcut:'CtrlAltS',system_audio:true},recording:false,transitioning:false,encoders:['Auto'],monitors:[{index:0,name:'Экран 1'}],error:'Запись остановилась: AcquireNextFrame failed: 887A0026',quick:null};
beforeEach(()=>{
  mocks.handlers.clear();mocks.invoke.mockReset();
  mocks.invoke.mockImplementation((name:string)=>Promise.resolve(name==='snapshot'?state:name==='library'?[]:undefined));
});
afterEach(()=>cleanup());
it('clears an initial capture error when recording recovers',async()=>{
  render(<App/>);expect((await screen.findByRole('alert')).textContent).toContain('887A0026');
  await act(async()=>mocks.handlers.get('state')?.({payload:{...state,recording:true,error:null}}));
  expect(screen.queryByRole('alert')).toBeNull();
});
it('clears a live capture error without hiding an independent import failure',async()=>{
  mocks.invoke.mockImplementation((name:string)=>name==='import_video'?Promise.reject('Не удалось открыть файл'):Promise.resolve(name==='snapshot'?{...state,error:null}:name==='library'?[]:undefined));
  render(<App/>);await act(async()=>{});
  await act(async()=>mocks.handlers.get('state')?.({payload:state}));
  expect(screen.getByRole('alert').textContent).toContain('887A0026');
  fireEvent.click(screen.getAllByRole('button',{name:'Добавить видео'})[0]);
  expect(await screen.findByText('Не удалось открыть файл')).toBeTruthy();
  await act(async()=>mocks.handlers.get('state')?.({payload:{...state,recording:true,error:null}}));
  expect(screen.getByRole('alert').textContent).toContain('Не удалось открыть файл');
});
