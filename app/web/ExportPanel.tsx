import {useEffect,useState,type ReactNode} from 'react';
import {Collapsible,CollapsibleTrigger,CollapsiblePanel} from '@ui-sdk/react/collapsible';
import {AudioLines,ChevronDown,Film,Layers,LoaderCircle,SlidersHorizontal,X} from 'lucide-react';
import {IconButton,Select,Slider,Switch} from './components';
import {api,discordSizeMb,withPreset,effectiveConfig,estimatedSize,type Clip,type Compression,type ExportConfig,type Format,type Snapshot,type Codec} from './api';

const compressionLabels:Record<Compression,string>={copy:'Без сжатия',balanced:'Баланс',compact:'Компактно',discord:`Discord · ${discordSizeMb} МБ`,custom:'Вручную'};

export function ExportPanel({clip,config,setConfig,start,end,onClose,onExport,busy,state,mode='save',embedded=false,advancedContent}:{clip:Clip;config:ExportConfig;setConfig:(c:ExportConfig)=>void;start:number;end:number;onClose:()=>void;onExport:()=>void;busy:boolean;state:Snapshot;mode?:'save'|'segment'|'movie';embedded?:boolean;advancedContent?:ReactNode}){
  config=effectiveConfig(clip,config);
  const copy=config.quality==='Copy';const gif=config.encoding.format==='Gif';const webm=config.encoding.format==='Webm';const hardware=state.encoders.some(e=>!['Auto','Cpu'].includes(e));
  const presets:Compression[]=mode==='save'&&!gif&&!webm?['copy','balanced','compact']:['balanced','compact'];
  if(mode!=='segment')presets.push('discord');
  const presetIndex=presets.indexOf(config.preset);
  const discord=config.preset==='discord';
  const compressionLabel=config.quality==='Target'&&!discord?'До размера файла':compressionLabels[config.preset];
  const changeFormat=(format:Format)=>{let next={...config,encoding:{...config.encoding,format,codec:(format==='Webm'?'Vp9':config.encoding.codec==='Vp9'?'H264':config.encoding.codec) as Codec}};if(format==='Gif'||format==='Webm')next.encoding.hardware=false;if((discord&&format!=='Mp4')||((format==='Gif'||format==='Webm')&&(copy||config.quality==='Target')))next=withPreset(next,'balanced',clip);if(format==='Gif'&&next.encoding.fps===null)next.encoding.fps=15;setConfig(next);};
  const estimated=estimatedSize(clip,config,end-start);
  const groups=Array.from(new Set(clip.audio_tracks.map(t=>t.name))).map(name=>({name,indices:clip.audio_tracks.filter(t=>t.name===name).map(t=>t.index)}));
  const [audioIcons,setAudioIcons]=useState<{path:string;icons:Record<string,string>}>();
  const showAudio=mode!=='movie'&&!gif&&groups.length>0;
  useEffect(()=>{
    if(!showAudio)return;
    let live=true;
    void (async()=>{
      try{
        const icons=await api.audioIcons(clip.path);
        if(live&&icons)setAudioIcons({path:clip.path,icons});
      }catch{}
    })();
    return()=>{live=false;};
  },[clip.path,showAudio]);
  return <aside className={`export-panel ${embedded?'embedded':''}`}>
    {!embedded&&<header className="panel-header"><h2>{mode==='movie'?'Экспорт':mode==='segment'?'Фрагмент':'Обработка'}</h2><IconButton label="Закрыть параметры" onClick={onClose}><X size={16}/></IconButton></header>}
    <div className="export-scroll"><fieldset disabled={busy}>
      {mode!=='segment'&&<div className="field"><label className="export-section-label"><Film size={15} aria-hidden="true"/>Формат</label><div className="segments formats">{(['Mp4','Mkv','Webm','Gif'] as const).map(format=><button key={format} className={config.encoding.format===format?'active':''} onClick={()=>changeFormat(format)}>{format==='Webm'?'WebM':format.toUpperCase()}</button>)}</div></div>}
      <div className="field">
        <div className="compression-heading"><span className="export-section-label"><Layers size={15} aria-hidden="true"/>Сжатие</span><span className="compression-status">{compressionLabel}</span></div>
        <Slider label="Сжатие" value={presetIndex<0?presets.indexOf('balanced'):presetIndex} valueText={compressionLabel} min={0} max={presets.length-1} disabled={busy} onChange={step=>setConfig(withPreset(config,presets[step],clip))}/>
      </div>
      {(!gif||mode!=='segment')&&<div className="rate-summary">
        {!gif&&!discord&&<span>≈ {estimated.toFixed(1)} МБ</span>}
        {mode!=='segment'&&<button type="button" className="discord-preset" aria-pressed={discord} onClick={()=>setConfig(withPreset(config,discord?'balanced':'discord',clip))}>{compressionLabels.discord}</button>}
      </div>}
      {showAudio&&<div className="field audio-tracks"><label className="export-section-label"><AudioLines size={15} aria-hidden="true"/>Звук</label>{groups.map(({name,indices})=>{
        const icon=audioIcons?.path===clip.path?audioIcons.icons[name]:undefined;
        return <div className="audio-track" key={name}>
          {icon?<img className="audio-track-icon" src={icon} alt=""/>:<AudioLines className="audio-track-icon" aria-hidden="true"/>}
          <span title={name}>{name}</span>
          <Switch label={name} disabled={busy} value={indices.every(i=>config.audioTracks.includes(i))} onChange={enabled=>setConfig({...config,audioTracks:enabled?[...new Set([...config.audioTracks,...indices])]:config.audioTracks.filter(i=>!indices.includes(i))})}/>
        </div>;
      })}</div>}
      <Collapsible className="advanced export-advanced"><CollapsibleTrigger className="disclosure"><SlidersHorizontal size={15} aria-hidden="true"/>Дополнительно<ChevronDown size={14} aria-hidden="true"/></CollapsibleTrigger><CollapsiblePanel className="advanced-content">
        {advancedContent}
        {hardware&&!webm&&!gif&&<div className="setting-row gpu-row"><label>Кодировать на GPU</label><Switch value={config.encoding.hardware} label="Кодировать на видеокарте" disabled={copy||config.quality==='Target'} onChange={value=>setConfig({...config,preset:mode==='segment'?'custom':config.preset,encoding:{...config.encoding,hardware:value}})}/></div>}
        <><div className="field"><label>Разрешение</label><Select value={config.height===null?'original':String(config.height)} disabled={copy} label="Разрешение экспорта" onChange={v=>setConfig({...config,height:v==='original'?null:Number(v)})} options={[{value:'original',label:`Исходное (${clip.width} × ${clip.height})`},...[2160,1440,1080,720,480].filter(h=>h<clip.height).map(h=>({value:String(h),label:`${h}p`}))]}/></div><div className="field"><label>Частота кадров</label><Select value={config.encoding.fps===null?'original':String(config.encoding.fps)} disabled={copy} label="FPS экспорта" onChange={v=>setConfig({...config,encoding:{...config.encoding,fps:v==='original'?null:Number(v)}})} options={[...(!gif?[{value:'original',label:`Исходная (${Math.round(clip.fps)} FPS)`}]:[]),...[15,24,30,60].map(fps=>({value:String(fps),label:`${fps} FPS`}))]}/></div></>
        <div className="field"><label>Кодек</label><Select value={config.encoding.codec} disabled={copy||gif||webm||config.quality==='Target'} label="Кодек экспорта" options={webm?[{value:'Vp9',label:'VP9'}]:[{value:'H264',label:'H.264 · совместимость'},{value:'Hevc',label:'H.265 / HEVC'}]} onChange={codec=>setConfig({...config,preset:'custom',encoding:{...config.encoding,codec}})}/></div>
        {!copy&&!gif&&<><div className="field"><label>Режим сжатия</label><Select value={config.automatic&&config.quality==='Bitrate'?'Auto':config.quality} label="Режим сжатия" options={[{value:'Auto',label:'Автоматически'},{value:'Bitrate',label:'Вручную'},...(!webm?[{value:'Target' as const,label:'До размера файла'}]:[])]} onChange={quality=>setConfig({...config,preset:quality==='Auto'?'balanced':'custom',automatic:quality==='Auto',quality:quality==='Auto'?'Bitrate':quality,encoding:{...config.encoding,codec:quality==='Target'?'H264':config.encoding.codec,hardware:quality==='Target'?false:config.encoding.hardware}})}/></div>
          {config.quality==='Target'?(!discord&&<div className="field"><label htmlFor="target-size">Размер, МБ</label><input id="target-size" type="number" min={1} max={2000} value={config.target} onChange={e=>setConfig({...config,target:Math.max(1,Number(e.target.value))})}/></div>):<div className="field"><div className="setting-heading"><label>Битрейт</label><output>{config.bitrate.toFixed(1)} Мбит/с</output></div><Slider label="Битрейт" value={config.bitrate} min={.5} max={100} step={.1} onChange={bitrate=>setConfig({...config,preset:'custom',automatic:false,bitrate})}/></div>}
        </>}
        {copy&&<p className="technical-note">Потоки сохраняются без перекодирования. Границы округляются по ключевым кадрам.</p>}
      </CollapsiblePanel></Collapsible>
    </fieldset></div>
    {!embedded&&<footer className="export-footer">{gif&&<div className="estimate">GIF · без звука</div>}{busy?<><div className="export-progress"><LoaderCircle size={15} className="spin"/>Сохранение…</div><button className="secondary-button" onClick={()=>void api.cancel()}>Отменить</button></>:<button className="primary-button" onClick={onExport}>{mode==='movie'?'Сохранить видео':mode==='segment'?'Готово':'Сохранить копию'}</button>}</footer>}
  </aside>;
}
