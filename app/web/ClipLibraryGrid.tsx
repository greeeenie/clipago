import {useCallback,useLayoutEffect,useRef,useState,type CSSProperties,type ReactNode,type RefObject} from 'react';
import {Menu,MenuContextTrigger,MenuContent,MenuItem} from '@ui-sdk/react/menu';
import {Dialog,DialogBackdrop,DialogContent,DialogTitle,DialogClose} from '@ui-sdk/react/dialog';
import {Popover,PopoverTrigger,PopoverContent,createPopoverController,type PopoverOptions} from '@ui-sdk/react/popover';
import {AppWindow,Check,ChevronLeft,ChevronRight,Copy,Film,FolderOpen,Gamepad2,Layers2,LoaderCircle,Play,Plus,Trash2,Video,X,Zap} from 'lucide-react';
import {api,clipDate,clipSourceLabel,clipTitle,localClipTime,time,type Clip,type Snapshot,type SourceFilter} from './api';
import {clipGroups} from './projectState';
import {Thumbnail,Select,type SelectOption} from './components';

type LibraryItem={key:string;clip:Clip;versions:Clip[];source?:string;date?:number};
type SourceFilters=Record<string,SourceFilter>;
type HoverPosition={x:number;y:number;scrolling:boolean;resumed?:boolean};
export function libraryItems(clips:Clip[]):LibraryItem[]{
 return [
  ...clipGroups(clips).map(({path,raw,edited})=>{
   const versions=[...(raw?[raw]:[]),...edited];
   const source=versions.find(clip=>clip.source?.trim())?.source?.trim();
   const clip=edited[0]??raw!;
   return {key:path,clip,versions,source,date:clipDate(raw??clip)};
  }),
  ...clips.filter(clip=>clip.is_movie).map(clip=>({key:clip.path,clip,versions:[clip],date:clipDate(clip)})),
 ].sort((a,b)=>(b.date??Number.MIN_SAFE_INTEGER)-(a.date??Number.MIN_SAFE_INTEGER));
}

const versionsPerPage=5;
const dateSections=['Сегодня','Вчера','Последние 7 дней','Последние 30 дней','Больше месяца назад','Без даты'];
const dayMs=86_400_000;
function librarySections(items:LibraryItem[]){
 const today=Math.floor(localClipTime(new Date())/dayMs);
 const groups=dateSections.map(title=>({title,items:[] as LibraryItem[]}));
 for(const item of items){
  const age=item.date===undefined?NaN:today-Math.floor(item.date/dayMs);
  const section=age<=0?0:age===1?1:age<7?2:age<30?3:Number.isFinite(age)?4:5;
  groups[section].items.push(item);
 }
 return groups.filter(group=>group.items.length>0);
}

function VersionStack({item,title,hover,renderCard}:{item:LibraryItem;title:string;hover:RefObject<HoverPosition>;renderCard:(clip:Clip,title:string,versions?:Clip[],source?:string)=>ReactNode}){
 const stack=useRef<HTMLDivElement>(null);
 const openedTop=useRef<number|null>(null);
 const [controller]=useState(()=>createPopoverController());
 const [raised,setRaised]=useState<string[]>([]);
 const [page,setPage]=useState(0);
 const [edge,setEdge]=useState<{side:'left'|'right';spacing:number;padding:number}|null>(null);
 const raise=(path:string)=>{
  setRaised(previous=>previous.at(-1)===path?previous:[...previous.filter(item=>item!==path),path]);
 };
 const mountSpread=useCallback((panel:HTMLDivElement|null)=>{
  const tile=stack.current;
  if(!panel||!tile)return;
  const scrollArea=tile.closest('.library-scroll,.picker-scroll');
  const dismiss=()=>{controller.close();};
  const hitsSpread=(target:Element|null)=>{
   const control=target?.closest('button.media-card,.version-spread-pages');
   return (control&&panel.contains(control))||target?.closest('.clip-menu[data-open]');
  };
  const followScroll=()=>{
   if(!scrollArea||openedTop.current===null)return;
   const region=scrollArea.getBoundingClientRect();
   const cover=tile.getBoundingClientRect();
   if(cover.bottom<=region.top||cover.top>=region.bottom){dismiss();return;}
   // Keep the popover's collision anchor fixed during scrolling so it cannot flip above its row.
   panel.style.translate=`0 ${cover.top-openedTop.current}px`;
   const bounds=panel.getBoundingClientRect();
   const overshootTop=Math.max(0,region.top-bounds.top);
   const overshootBottom=Math.max(0,bounds.bottom-region.bottom);
   panel.style.clipPath=overshootTop||overshootBottom?`inset(${overshootTop}px -100vw ${overshootBottom}px)`:'';
   const {x,y,scrolling}=hover.current;
   if(!scrolling||!panel.hasAttribute('data-spread-settled')||!Number.isFinite(x)||!Number.isFinite(y))return;
   const focused=document.activeElement;
   if(focused&&panel.contains(focused)&&focused.matches(':focus-visible'))return;
   if(!hitsSpread(document.elementFromPoint(x,y)))dismiss();
  };
  const scrollThrough=(event:WheelEvent)=>{
   hover.current={x:event.clientX,y:event.clientY,scrolling:true};
   if(scrollArea&&event.target instanceof Node&&panel.contains(event.target)&&!event.ctrlKey){
    event.preventDefault();
    const units=event.deltaMode===WheelEvent.DOM_DELTA_PAGE?scrollArea.clientHeight:event.deltaMode===WheelEvent.DOM_DELTA_LINE?16:1;
    scrollArea.scrollBy({top:event.deltaY*units,left:event.deltaX*units});
   }
  };
  const leaveCards=(event:PointerEvent)=>{
   if(event.pointerType!=='mouse')return;
   const point=hover.current;
   if(point.scrolling){
    if(point.x===event.clientX&&point.y===event.clientY)return;
    hover.current={x:event.clientX,y:event.clientY,scrolling:false,resumed:true};
   }
   const target=event.buttons?document.elementFromPoint(event.clientX,event.clientY):event.target;
   if(target instanceof Element&&hitsSpread(target))return;
   if(!panel.hasAttribute('data-spread-settled')){
    const cards=panel.hasAttribute('data-spread-ready')?[...panel.querySelectorAll('button.media-card')]:[tile];
    const overCard=cards.some(card=>{
     const rect=card.getBoundingClientRect();
     return event.clientX>=rect.left&&event.clientX<rect.right&&event.clientY>=rect.top&&event.clientY<rect.bottom;
    });
    if(overCard)return;
   }
   dismiss();
  };
  document.addEventListener('pointermove',leaveCards,true);
  document.addEventListener('wheel',scrollThrough,{capture:true,passive:false});
  scrollArea?.addEventListener('scroll',followScroll,{passive:true});
  window.addEventListener('resize',dismiss);
  const animations:Animation[]=[];
  let active=true;
  let frame=requestAnimationFrame(()=>{
   frame=requestAnimationFrame(()=>{
    const cover=tile.querySelector(':scope > .media-card')?.getBoundingClientRect();
    const backing=tile.querySelector('.clip-stack-back')?.getBoundingClientRect();
    const slots=Array.from(panel.querySelectorAll<HTMLElement>('.version-spread-card'));
    const leading=slots[0]?.getBoundingClientRect();
    if(cover&&leading&&!matchMedia('(prefers-reduced-motion: reduce)').matches){
     const travelX=leading.x-cover.x;
     const travelY=leading.y-cover.y;
     const moveTogether=Math.hypot(travelX,travelY)>45;
     slots.forEach((slot,index)=>{
      const card=slot.querySelector('.media-card');
      const destination=slot.getBoundingClientRect();
      if(!card||!destination.width||!destination.height)return;
      const source=index===0?cover:backing??cover;
      const scaleX=source.width/destination.width;
      const scaleY=source.height/destination.height;
      const style=getComputedStyle(card);
      const pivotX=parseFloat(style.transformOrigin);
      const x=source.left-destination.left+pivotX*(scaleX-1);
      const y=source.top-destination.top;
      const transform=(dx:number,dy:number)=>`translate(${x+dx}px,${y+dy}px) scale(${scaleX},${scaleY})`;
      const keyframes:Keyframe[]=[{transform:transform(0,0)}];
      if(moveTogether)keyframes.push({transform:transform(travelX*.65,travelY*.65),offset:.4});
      keyframes.push({transform:style.transform});
      animations.push(card.animate(keyframes,{
       duration:moveTogether?330:250,
       delay:moveTogether?0:Math.min(index,2)*24,
       easing:'cubic-bezier(.2,.7,.3,1)',
       fill:'backwards',
      }));
     });
    }
    tile.dataset.spreadReady='';
    panel.dataset.spreadReady='';
    void Promise.allSettled(animations.map(animation=>animation.finished)).then(()=>{
     if(active){panel.dataset.spreadSettled='';if(hover.current.scrolling)followScroll();}
    });
   });
  });
  return()=>{
   active=false;
   document.removeEventListener('pointermove',leaveCards,true);
   document.removeEventListener('wheel',scrollThrough,true);
   scrollArea?.removeEventListener('scroll',followScroll);
   window.removeEventListener('resize',dismiss);
   cancelAnimationFrame(frame);
   animations.forEach(animation=>animation.cancel());
   delete tile.dataset.spreadReady;
  };
 },[controller,hover]);
 const original=item.versions.find(clip=>!clip.original_path);
 const edits=item.versions.filter(clip=>clip.original_path);
 const ordered=[
  ...edits.map((clip,index)=>({clip,label:clip.title??`V${edits.length-index}`})),
  ...(original?[{clip:original,label:original.title??'Исходник'}]:[]),
 ];
 const front=ordered[0];
 const pageSize=versionsPerPage;
 const columns=Math.min(ordered.length,pageSize);
 const pageCount=Math.ceil(ordered.length/pageSize);
 const pageIndex=Math.min(page,pageCount-1);
 const start=pageIndex*pageSize;
 const visibleCards=ordered.slice(start,start+pageSize);
 const highlightedPath=raised.at(-1)??visibleCards[0]?.clip.path;
 const changePage=(next:number)=>{setRaised([]);setPage(next);};
 const defaultSpacing=.64;
 const sidePadding=24;
 const collisionPadding=20;
 const leftward=edge?.side==='right';
 const spacing=edge?.spacing??defaultSpacing;
 const leftPadding=edge?.side==='left'?edge.padding:sidePadding;
 const rightPadding=edge?.side==='right'?edge.padding:sidePadding;
 const openSpread:NonNullable<PopoverOptions['onOpenChange']>=(open,details)=>{
  if(details.reason==='trigger-hover'&&hover.current.scrolling){details.cancel();return;}
  if(!open)return;
  const tile=stack.current;
  if(!tile)return;
  const rect=tile.getBoundingClientRect();
  const region=tile.closest('.library-scroll,.picker-scroll')?.getBoundingClientRect();
  const top=region?.top??0;
  const bottom=Math.min(window.innerHeight-collisionPadding,region?.bottom??window.innerHeight);
  const height=rect.height+(pageCount>1?72:24);
  if(rect.top<top-1||rect.top+height>bottom+1){details.cancel();return;}
  openedTop.current=rect.top;
  changePage(0);
  const leftSpace=rect.left-(region?.left??0);
  const rightSpace=(region?.right??window.innerWidth)-rect.right;
  const atLeft=leftSpace<rect.width/2&&rightSpace>=rect.width/2;
  const atRight=rightSpace<rect.width/2&&leftSpace>=rect.width/2;
  if(!atLeft&&!atRight){setEdge(null);return;}

  // Keep the cover full size and fit the other cards in the space toward the center.
  const available=(atLeft?window.innerWidth-rect.left:rect.right)-sidePadding-collisionPadding;
  const gutter=atLeft?rect.left:window.innerWidth-rect.right;
  setEdge({
   side:atLeft?'left':'right',
   spacing:Math.min(defaultSpacing,Math.max(0,(available/rect.width-1)/(columns-1))),
   padding:Math.max(0,Math.min(sidePadding,gutter-collisionPadding)),
  });
 };
 const spreadWidth=1+(columns-1)*spacing;
 const anchor={getBoundingClientRect:()=>{
  const tile=stack.current;
  const rect=tile?.getBoundingClientRect();
  if(!rect)return {x:0,y:0,width:0,height:0};
  const region=tile?.closest('.library-scroll,.picker-scroll')?.getBoundingClientRect();
  const leftSpace=rect.left-(region?.left??0);
  const rightSpace=(region?.right??window.innerWidth)-rect.right;
  const interior=Math.min(leftSpace,rightSpace)>rect.width/2;
  const panelWidth=Math.min(window.innerWidth-2*collisionPadding,rect.width*spreadWidth+leftPadding+rightPadding);
  const x=leftward?rect.right-rect.width*spreadWidth:interior?rect.left+(rect.width-panelWidth)/2+leftPadding:rect.left;
  return {x,y:openedTop.current??rect.top,width:rect.width,height:0};
 }};
 return <div ref={stack} className="clip-tile clip-stack" data-paged={pageCount>1?'':undefined} onPointerMove={event=>{
  if(event.pointerType!=='mouse'||!hover.current.resumed||!(event.target instanceof Element))return;
  const cover=event.target.closest('button.media-card');
  if(cover?.parentElement!==event.currentTarget)return;
  hover.current.resumed=false;
  if(!controller.isOpen())controller.setOpen(true,{reason:'trigger-hover',event:event.nativeEvent});
 }}>
  <div className="clip-stack-back" aria-hidden="true"/>
  <Popover controller={controller} openOnHover delay={130} closeDelay={0} onOpenChange={openSpread}>
   {renderCard(item.clip,title,item.versions,item.source)}
   <PopoverContent ref={mountSpread} className="version-spread" data-edge={edge?.side} aria-label={`Версии клипа ${title}`} anchor={anchor} align="start" alignOffset={-leftPadding} sideOffset={0} collisionPadding={collisionPadding} style={{'--spread-width':spreadWidth,'--spread-overlap':`${(1-spacing)*100}%`,'--spread-left-padding':leftPadding+'px','--spread-right-padding':rightPadding+'px'} as CSSProperties}>
    <div className="version-spread-row">
     {visibleCards.map(({clip,label},index)=>{
      const offset=(index-(visibleCards.length-1)/2)*(leftward?-1:1);
      const isFront=clip===front.clip;
      const raisedIndex=raised.indexOf(clip.path);
      const style={
       zIndex:raisedIndex<0?visibleCards.length-index:pageSize+1+raisedIndex,
       '--spread-angle':`${edge&&index===0?0:offset*Math.min(4,12/Math.max(1,visibleCards.length-1))}deg`,
       '--spread-pivot':offset<0?'top right':'top left',
      } as CSSProperties;
      return <div className="version-spread-card" data-highlighted={highlightedPath===clip.path?'':undefined} key={clip.path} style={style}
       onPointerMove={event=>{
        if(event.pointerType==='mouse'&&!hover.current.scrolling&&highlightedPath!==clip.path&&event.currentTarget.closest('[data-spread-settled]'))raise(clip.path);
       }}
       onFocus={event=>{if(event.target.matches(':focus-visible'))raise(clip.path);}}
      >
       {renderCard(clip,isFront?title:label,undefined,item.source)}
      </div>;
     })}
    </div>
    {pageCount>1&&<div className="version-spread-pages">
     <button type="button" className="icon-button" aria-label="Предыдущие версии" disabled={pageIndex===0} onClick={()=>changePage(pageIndex-1)}><ChevronLeft size={16}/></button>
     <span aria-live="polite">{start+1}–{start+visibleCards.length} из {ordered.length}</span>
     <button type="button" className="icon-button" aria-label="Следующие версии" disabled={pageIndex===pageCount-1} onClick={()=>changePage(pageIndex+1)}><ChevronRight size={16}/></button>
    </div>}
   </PopoverContent>
  </Popover>
 </div>;
}
function Cards({items,sourceFilters,onOpen,onCopy,onDelete,picking=false,chosen=[],onToggle}:{items:LibraryItem[];sourceFilters?:SourceFilters;onOpen?:(c:Clip)=>void;onCopy?:(c:Clip)=>void;onDelete?:(c:Clip)=>void;picking?:boolean;chosen?:string[];onToggle?:(path:string)=>void}){
 const root=useRef<HTMLDivElement>(null);
 const hover=useRef<HoverPosition>({x:NaN,y:NaN,scrolling:false});
 useLayoutEffect(()=>{
  const scroller=root.current?.closest('.library-scroll,.picker-scroll');
  const suspend=()=>{hover.current.scrolling=true;};
  scroller?.addEventListener('scroll',suspend,{passive:true});
  return()=>scroller?.removeEventListener('scroll',suspend);
 },[]);
 const renderCard=(clip:Clip,title:string,versions:Clip[]=[],sourceKey=clip.source??undefined)=>{
  const source=sourceKey?sourceFilters?.[sourceKey]:undefined;
  const stacked=versions.length>1;
  const selected=stacked?versions.some(version=>chosen.includes(version.path)):chosen.includes(clip.path);
  const card=<button
   className={`media-card ${selected?'chosen':''}`}
   aria-pressed={picking&&!stacked?selected:undefined}
   aria-label={stacked?`Версии клипа ${title}`:picking?`Выбрать ${title}`:`Открыть ${title}`}
   onPointerDown={event=>{
    if(event.pointerType!=='mouse'||event.button!==0)return;
    const button=event.currentTarget;
    if((stacked&&!picking)||button.closest('.version-spread:not([data-spread-settled])')){
     // A moving card can slide out from under the pointer before the click ends.
     button.setPointerCapture(event.pointerId);
    }
   }}
   onClickCapture={stacked&&!picking?event=>{
    const pointer=event.nativeEvent;
    if(!(pointer instanceof PointerEvent)||pointer.pointerType!=='mouse')return;
    event.preventDefault();
    event.stopPropagation();
    onOpen?.(clip);
   }:undefined}
   onClick={stacked?undefined:()=>picking?onToggle?.(clip.path):onOpen?.(clip)}
  >
   <div className="media-cover">
    <Thumbnail clip={clip} seconds={Math.min(2,clip.duration*.2)}/>
    <span className="media-cover-title" title={title}>
     {source?.icon?<img src={source.icon} alt=""/>:source?.game||clip.game?<Gamepad2 size={14} aria-hidden="true"/>:sourceKey?<AppWindow size={14} aria-hidden="true"/>:null}
     <span>{title}</span>
    </span>
    <span className="duration">{time(clip.duration)}</span>
    {(!clip.viewed||versions.some(version=>!version.viewed))&&!clip.is_movie&&<span className="unread-dot" aria-label="Не просмотрен"/>}
    {picking&&<span className="pick-check">{selected&&<Check size={14}/>}</span>}
    {stacked&&<span className="version-count" title={`Версий: ${versions.length}`}><Layers2 size={13} aria-hidden="true"/>{versions.length}</span>}
   </div>
  </button>;
  const trigger=stacked?<PopoverTrigger render={card}/>:card;
  if(!onCopy||!onDelete)return trigger;
  return <Menu>
   <MenuContextTrigger render={trigger}/>
   <MenuContent className="dropdown clip-menu" aria-label="Действия с клипом">
    <MenuItem value="open" onSelect={()=>onOpen?.(clip)}><Play size={15}/>Открыть</MenuItem>
    <MenuItem value="copy" onSelect={()=>onCopy(clip)}><Copy size={15}/>Скопировать</MenuItem>
    <MenuItem value="delete" className="danger-action" onSelect={()=>onDelete(clip)}><Trash2 size={15}/>Удалить</MenuItem>
   </MenuContent>
  </Menu>;
 };
 return <div ref={root} className="library-sections" onPointerMoveCapture={event=>{
  if(event.pointerType!=='mouse')return;
  const point=hover.current;
  if(point.x!==event.clientX||point.y!==event.clientY)hover.current={x:event.clientX,y:event.clientY,scrolling:false,resumed:point.scrolling};
 }}>{librarySections(items).map(section=><section className="library-section" key={section.title}>
  <h2>{section.title}</h2>
  <div className="library-grid">{section.items.map(item=>{
  const {key,clip,versions}=item;
  const source=item.source?sourceFilters?.[item.source]:undefined;
  const title=clipTitle(clip,source?.game?source.name:clipSourceLabel(item.source));
  return versions.length>1
   ?<VersionStack key={key} item={item} title={title} hover={hover} renderCard={renderCard}/>
   :<div className="clip-tile" key={key}>{renderCard(clip,title,undefined,item.source)}</div>;
  })}</div>
 </section>)}</div>;
}
export function ClipLibraryGrid({items,sourceFilters,status,onRetry,sourceFilter,sourceOptions,onSourceFilter,onOpen,onCopy,onDelete,onImport,onCapture,onError,state,scrollPosition}:{items:LibraryItem[];sourceFilters?:SourceFilters;status:'loading'|'ready'|'failed';onRetry:()=>void;sourceFilter:string;sourceOptions:SelectOption[];onSourceFilter:(value:string)=>void;onOpen:(c:Clip)=>void;onCopy:(c:Clip)=>void;onDelete:(c:Clip)=>void;onImport:()=>void;onCapture:()=>void;onError:(e:string)=>void;state?:Snapshot;scrollPosition:RefObject<number>}){
 const host=useRef<HTMLDivElement>(null);useLayoutEffect(()=>{if(host.current)host.current.scrollTop=scrollPosition.current;},[scrollPosition,sourceFilter]);
 const count=items.length;const replayStatus=state?.replay_status??(state?.transitioning?'warming':state?.recording?'ready':'stopped');const captureLabel=replayStatus==='saving'?'Сохранение…':replayStatus==='warming'?'Подготовка…':replayStatus==='ready'?'Сохранить момент':'Включить запись';
 return <main className="library-page">
  <header className="page-header">
   <div className="library-title">
    <h1>Клипы</h1>{count>0&&<span className="item-count" title={`Клипов: ${count}`}>{count}</span>}
    {sourceOptions.length>1&&<Select value={sourceFilter} options={sourceOptions} label="Фильтр по источнику" onChange={onSourceFilter}/>}
   </div>
   <div className="library-actions">
    <button className="secondary-button" onClick={()=>void api.folder().catch(e=>onError(String(e)))}><FolderOpen size={16}/>Открыть папку</button>
    {count>0&&<button className="primary-button" onClick={onImport}><Plus size={16}/>Добавить видео</button>}
   </div>
  </header>
  <div className="library-scroll" ref={host} onScroll={e=>{scrollPosition.current=e.currentTarget.scrollTop;}}>
   {count?<Cards items={items} sourceFilters={sourceFilters} onOpen={onOpen} onCopy={onCopy} onDelete={onDelete}/>
   :status==='loading'?<div className="empty-state library-status" role="status"><LoaderCircle size={20} className="spin" aria-hidden="true"/>Загрузка клипов…</div>
   :status==='failed'?<div className="empty-state library-status"><span>Не удалось загрузить клипы</span><button className="secondary-button" onClick={onRetry}>Повторить</button></div>
   :<div className="empty-library">
    <div className="empty-film" aria-hidden="true"><span><Film size={28} strokeWidth={1.5}/></span></div>
    <h2>Пока нет клипов</h2>
    <div className="empty-actions">
     <button className="primary-button" onClick={onImport}><Plus size={16}/>Добавить видео</button>
     <button className="secondary-button" onClick={onCapture} disabled={!state||replayStatus==='warming'||replayStatus==='saving'}>
      {replayStatus==='ready'?<Zap size={16}/>:<Video size={16}/>}
      {captureLabel}
     </button>
    </div>
   </div>}
  </div>
 </main>;
}
export function ClipPicker({open,onOpenChange,clips,sourceFilters,onAdd,onImport,limit=128}:{open:boolean;onOpenChange:(v:boolean)=>void;clips:Clip[];sourceFilters?:SourceFilters;onAdd:(clips:Clip[])=>void;onImport:()=>void;limit?:number}){
 const [chosen,setChosen]=useState<string[]>([]);const toggle=(path:string)=>setChosen(v=>v.includes(path)?v.filter(p=>p!==path):v.length<limit?[...v,path]:v);const close=()=>{setChosen([]);onOpenChange(false);};
 return <Dialog open={open} onOpenChange={v=>{setChosen([]);onOpenChange(v);}}><DialogBackdrop className="dialog-backdrop"/><DialogContent className="clip-picker"><header className="dialog-header"><DialogTitle>Добавить клипы</DialogTitle><span className="spacer"/><button className="secondary-button" onClick={onImport}><Plus size={16}/>Импорт</button><DialogClose className="icon-button" aria-label="Закрыть выбор клипов"><X size={16}/></DialogClose></header><div className="picker-scroll">{clips.length?<Cards items={libraryItems(clips)} sourceFilters={sourceFilters} picking chosen={chosen} onToggle={toggle}/>:<div className="picker-empty"><button className="primary-button" onClick={onImport}><Plus size={16}/>Добавить видео</button></div>}</div><footer className="picker-footer"><button className="secondary-button" onClick={close}>Отмена</button><button className="primary-button" disabled={!chosen.length} onClick={()=>{onAdd(chosen.map(path=>clips.find(c=>c.path===path)!).filter(Boolean));setChosen([]);onOpenChange(false);}}>Добавить{chosen.length?` ${chosen.length}`:''}</button></footer></DialogContent></Dialog>;
}
