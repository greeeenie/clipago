import type {PointerEvent,MouseEvent,RefObject} from 'react';
export type ClipDrop={kind:'clip'|'segment';value:string;label:string};
let current:{payload:ClipDrop;id:number;x:number;y:number;active:boolean;ghost?:HTMLDivElement;target?:HTMLElement}|undefined;
let blockClick=false;
function clear(){current?.ghost?.remove();current?.target?.classList.remove('clip-drop-target');current=undefined;}
// Native Tauri file-drop remains enabled for Explorer imports. Pointer capture
// handles internal drags without competing with WebView2's native OLE handler.
export function clipDrag(payload:ClipDrop,disabled=false,options?:{scrollRef?:RefObject<HTMLDivElement|null>;onDrop?:(payload:ClipDrop,index:number)=>void}){
 return {
  draggable:false,
  onPointerDown:(event:PointerEvent<HTMLElement>)=>{if(disabled||event.button!==0)return;clear();current={payload,id:event.pointerId,x:event.clientX,y:event.clientY,active:false};event.currentTarget.setPointerCapture?.(event.pointerId);},
  onPointerMove:(event:PointerEvent<HTMLElement>)=>{
   if(!current||event.pointerId!==current.id)return;
   if(!current.active&&Math.hypot(event.clientX-current.x,event.clientY-current.y)<7)return;
   event.preventDefault();current.active=true;
   if(!current.ghost){current.ghost=document.createElement('div');current.ghost.className='clip-drag-ghost';current.ghost.textContent=current.payload.label;document.body.append(current.ghost);}
   current.ghost.style.left=`${event.clientX+12}px`;current.ghost.style.top=`${event.clientY+12}px`;
   const target=document.elementFromPoint(event.clientX,event.clientY)?.closest<HTMLElement>('[data-movie-drop]');
   if(current.target!==target){current.target?.classList.remove('clip-drop-target');target?.classList.add('clip-drop-target');current.target=target??undefined;}
   const sequence=options?.scrollRef?.current;if(sequence){const rect=sequence.getBoundingClientRect();if(event.clientY>=rect.top&&event.clientY<=rect.bottom){if(event.clientX>rect.right-32)sequence.scrollLeft+=16;else if(event.clientX<rect.left+32)sequence.scrollLeft-=16;}}
  },
  onPointerUp:(event:PointerEvent<HTMLElement>)=>{
   if(!current||event.pointerId!==current.id)return;
   const {active,payload}=current;const target=current.target;
   if(active){event.preventDefault();blockClick=true;globalThis.setTimeout(()=>{blockClick=false;},0);if(target){const index=Number(target.dataset.movieDrop);if(options?.onDrop)options.onDrop(payload,index);else window.dispatchEvent(new CustomEvent('rewind-clip-drop',{detail:{...payload,index}}));}}
   clear();
  },
  onPointerCancel:clear,
  onClickCapture:(event:MouseEvent)=>{if(blockClick){event.preventDefault();event.stopPropagation();blockClick=false;}},
 };
}
