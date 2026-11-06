import type {ReactNode} from 'react';
import {ExportPanel} from './ExportPanel';
import type {Clip,ExportConfig,Snapshot} from './api';
import type {Selection} from './Timeline';

export function Inspector({clip,selection,config,setConfig,state,busy=false,children,segment=false}:{clip:Clip;selection:Selection;config:ExportConfig;setConfig:(c:ExportConfig)=>void;state:Snapshot;busy?:boolean;children:ReactNode;segment?:boolean}){
 return <aside className="inspector" aria-label={segment?'Обработка фрагмента':'Обработка клипа'}><ExportPanel embedded clip={clip} config={config} setConfig={setConfig} start={selection.start} end={selection.end} state={state} busy={busy} onClose={()=>{}} onExport={()=>{}} mode={segment?'segment':'save'}/><footer className="inspector-footer">{children}</footer></aside>;
}
