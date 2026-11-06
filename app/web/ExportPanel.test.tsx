// @vitest-environment jsdom
import {afterEach,expect,it,vi} from 'vitest';
import {cleanup,fireEvent,render,screen} from '@testing-library/react';
import {Tooltip} from 'radix-ui';
import {ExportPanel} from './ExportPanel';
import {api,defaultConfig,type Clip,type Snapshot} from './api';
const mocks=vi.hoisted(()=>({invoke:vi.fn().mockResolvedValue(null)}));
vi.mock('@tauri-apps/api/core',()=>({invoke:mocks.invoke,convertFileSrc:(path:string)=>path}));
const clip:Clip={original_path:null,viewed:false,is_movie:false,path:'clip.mp4',duration:45,width:1920,height:1080,bytes:20_000_000,has_audio:true,source:'Deadlock',fps:60,aspect_ratio:16/9,audio_mix_index:0,audio_tracks:[{index:1,name:'Deadlock'},{index:2,name:'Discord'},{index:3,name:'Discord'}]};
afterEach(cleanup);
it('groups application processes and removes every Discord stream together',()=>{
 const change=vi.fn();const config=defaultConfig(clip,false);
 render(<Tooltip.Provider><ExportPanel clip={clip} config={config} setConfig={change} start={0} end={45} onClose={()=>{}} onExport={()=>{}} busy={false} state={{encoders:['Cpu']} as Snapshot}/></Tooltip.Provider>);
 expect(screen.getAllByRole('checkbox')).toHaveLength(2);
 fireEvent.click(screen.getByRole('checkbox',{name:'Discord'}));
 expect(change.mock.calls[0][0].audioTracks).toEqual([1]);
});
it('keeps explicit mute in the export request',async()=>{
 const config=defaultConfig(clip,false);config.audioTracks=[];
 await api.export(clip,0,45,config);
 expect(mocks.invoke).toHaveBeenCalledWith('export_video',expect.objectContaining({request:expect.objectContaining({audio_tracks:[]})}));
});
