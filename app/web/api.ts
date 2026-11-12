import {invoke, convertFileSrc} from '@tauri-apps/api/core';
export type Encoder='Auto'|'Nvidia'|'Intel'|'Amd'|'Cpu';
export type EditorHotkeys={split:string;duplicate:string;delete:string;snapping:string;inspector:string;export:string;undo:string;redo:string;play:string;zoom_in:string;zoom_out:string};
export type Settings={buffer_seconds:number;fps:number;monitor:number;encoder:Encoder;output_dir:string;ffmpeg:string;ffprobe:string;shortcut:string;system_audio:boolean;microphone?:boolean;editor_hotkeys?:EditorHotkeys};
export type Clip={path:string;duration:number;width:number;height:number;bytes:number;has_audio:boolean;audio_tracks:{index:number;name:string}[];audio_mix_index:number|null;source:string|null;game?:boolean;fps:number;aspect_ratio:number;original_path:string|null;title?:string|null;viewed:boolean;is_movie:boolean};
export type SourceFilter={name:string;icon:string|null;game:boolean};
export type Snapshot={profile_id?:string;settings:Settings;recording:boolean;transitioning:boolean;replay_status?:'stopped'|'warming'|'ready'|'saving';encoders:Encoder[];monitors:{index:number;name:string}[];error:string|null;quick:Clip[]};
export type Format='Mp4'|'Mkv'|'Webm'|'Gif';
export type Codec='H264'|'Hevc'|'Vp9';
export type ExportOptions={format:Format;codec:Codec;fps:number|null;hardware:boolean};
export type Compression='copy'|'balanced'|'compact'|'custom';
export type ExportConfig={automatic:boolean;audioTracks:number[];preset:Compression;quality:'Bitrate'|'Target'|'Copy';bitrate:number;target:number;height:number|null;encoding:ExportOptions};
export const api={
  snapshot:()=>invoke<Snapshot>('snapshot'),
  defaultSettings:()=>invoke<Settings>('default_settings'),
  library:()=>invoke<Clip[]>('library'),
  viewed:(path:string)=>invoke<Clip>('mark_viewed',{path}),
  movie:(segments:MovieSegment[],clips:Clip[],output:ExportConfig,summary:Clip,name:string)=>invoke<Clip|null>('render_movie',{segments:segments.map(s=>({path:s.path,start:s.start,end:s.end,gap_before:s.gapBefore??0,...renderConfig(clips.find(c=>c.path===s.path)!,s.config),preprocess:s.config.preset==='compact'||s.config.preset==='custom'||!s.config.automatic||s.config.quality==='Target'})),output:{...renderConfig(summary,output),audio_tracks:undefined,name}}),
  import:()=>invoke<Clip|null>('import_video'),
  settings:(settings:Settings)=>invoke('update_settings',{settings}),
  record:(enabled:boolean)=>invoke('set_recording',{enabled}),
  saveReplay:()=>invoke('save_replay'),
  folder:()=>invoke('open_folder'),
  copyClip:(path:string)=>invoke<void>('copy_clip',{path}),
  deleteClip:(path:string)=>invoke<void>('delete_clip',{path}),
  renameClip:(path:string,title:string)=>invoke<Clip>('rename_clip',{path,title}),
  sourceFilters:()=>invoke<Record<string,SourceFilter>>('source_filters'),
  revealClip:(path:string)=>invoke('reveal_clip',{path}),
  openHelp:(page:'github'|'docs')=>invoke<void>('open_help_page',{page}),
  chooseFolder:()=>invoke<string|null>('choose_folder'),
  dismiss:()=>invoke('dismiss_error'),
  thumbnail:(path:string,seconds:number)=>invoke<string>('thumbnail',{path,seconds}),
  preview:(path:string)=>invoke<string>('prepare_preview',{path}),
  previewAudio:(path:string)=>invoke<{index:number;path:string}[]>('prepare_audio_preview',{path}),
  audioIcons:(path:string)=>invoke<Record<string,string>>('audio_track_icons',{path}),
  cancel:()=>invoke('cancel_export'),
  quick:(path:string)=>invoke('show_quick_clip',{path}),
  importPaths:(paths:string[])=>invoke<Clip[]>('import_paths',{paths}),
  export:(clip:Clip,start:number,end:number,config:ExportConfig,title?:string,copy=false)=>invoke<Clip|null>('export_video',{request:{path:clip.path,start,end,...renderConfig(clip,config)},title,copy}),
};
export const fileUrl=convertFileSrc;
export const filename=(path:string)=>path.split(/[\\/]/).pop()||path;
const replayDate=new Intl.DateTimeFormat('ru-RU',{day:'numeric',month:'long',timeZone:'UTC'});
export function clipSourceLabel(value:string|null|undefined){
  const source=value?.trim();
  return source&&source===source.toLowerCase()?source.replace(/^\p{L}/u,letter=>letter.toUpperCase()):source;
}
export function clipTitle(clip:Clip){
  if(clip.title)return clip.title;
  const name=filename(clip.original_path??clip.path);
  const dot=name.lastIndexOf('.');
  const stem=dot>0?name.slice(0,dot):name;
  const stamp=/^Replay_(\d{4}-\d{2}-\d{2})_(\d{2})-(\d{2})-\d{2}-\d{3}_[a-f0-9]{4}$/i.exec(stem);
  if(!stamp)return stem;
  const [,day,hours,minutes]=stamp;
  // Capture names store local wall time. Format that date without shifting it to another day.
  const date=new Date(`${day}T${hours}:${minutes}:00Z`);
  if(!Number.isFinite(date.getTime())||date.toISOString().slice(0,10)!==day)return stem;
  return `${clipSourceLabel(clip.source)||'Клип'} — ${replayDate.format(date)}, ${hours}:${minutes}`;
}
export function time(value:number,precise=false){const ms=Math.max(0,Math.round(value*1000));const base=`${Math.floor(ms/60000).toString().padStart(2,'0')}:${Math.floor(ms/1000%60).toString().padStart(2,'0')}`;return precise?`${base}.${(ms%1000).toString().padStart(3,'0')}`:base;}
export const snapBuffer=(n:number)=>Math.round(Math.max(30,Math.min(120,n))/5)*5;
export const frameTime=(n:number,fps:number)=>Math.round(n*Math.max(1,fps))/Math.max(1,fps);
export function trimAt(kind:'start'|'end'|'seek',time:number,start:number,end:number,duration:number,fps:number){
  const frame=1/Math.max(1,fps);const t=Math.max(0,Math.min(duration,frameTime(time,fps)));
  if(kind==='start')return {start:Math.min(t,Math.max(0,end-frame)),end,cursor:Math.min(t,Math.max(0,end-frame))};
  if(kind==='end')return {start,end:Math.max(t,Math.min(duration,start+frame)),cursor:Math.max(t,Math.min(duration,start+frame))};
  return {start,end,cursor:t};
}
export function recommendedBitrate(clip:Clip,config:Pick<ExportConfig,'height'|'encoding'|'preset'>){
  const height=Math.min(clip.height,config.height??clip.height);const width=clip.width*height/Math.max(1,clip.height);const fps=config.encoding.fps??clip.fps;
  const codec=config.encoding.codec==='Hevc'?.65:config.encoding.codec==='Vp9'?.75:1;
  return Math.round(Math.max(.5,Math.min(100,width*height*30*Math.pow(Math.max(1,fps)/30,.7)*.09/1e6*codec*(config.preset==='compact'?.65:1)))*10)/10;
}
export function effectiveConfig(clip:Clip,config:ExportConfig):ExportConfig{return config.automatic&&config.quality==='Bitrate'?{...config,bitrate:recommendedBitrate(clip,config)}:config;}
export function renderConfig(clip:Clip,raw:ExportConfig){const config=effectiveConfig(clip,raw);return {quality:config.quality,bitrate_mbps:config.bitrate,target_mb:config.target,max_height:config.height,encoding:config.encoding,audio_tracks:config.audioTracks};}
export function estimatedSize(clip:Clip,raw:ExportConfig,duration:number){const c=effectiveConfig(clip,raw);return c.quality==='Copy'?clip.bytes/Math.max(.01,clip.duration)*duration/1e6:c.quality==='Target'?c.target:(c.bitrate+(c.audioTracks.length?.128:0))*duration/8*1.02;}
export function defaultConfig(clip:Clip,hardware:boolean):ExportConfig{
  const config:ExportConfig={automatic:true,audioTracks:clip.audio_tracks.map(t=>t.index),preset:'balanced',quality:'Bitrate',bitrate:5,target:25,height:null,encoding:{format:'Mp4',codec:'H264',fps:null,hardware}};
  return effectiveConfig(clip,config);
}
export function withPreset(config:ExportConfig,preset:Compression,clip:Clip):ExportConfig{
  if(preset==='copy')return {...config,preset,quality:'Copy',height:null,encoding:{...config.encoding,fps:null,hardware:false}};
  return effectiveConfig(clip,{...config,preset,automatic:true,quality:'Bitrate'});
}
export type MovieSegment={id:string;path:string;start:number;end:number;config:ExportConfig;gapBefore?:number};
export type MovieProject={name:string;segments:MovieSegment[];output?:ExportConfig;snapping?:boolean};
