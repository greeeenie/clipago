import type {EditorHotkeys,Settings} from './api';

export const defaultEditorHotkeys:EditorHotkeys={split:'KeyB',duplicate:'Ctrl+KeyD',delete:'Delete',snapping:'KeyM',inspector:'KeyP',export:'Ctrl+KeyE',undo:'Ctrl+KeyZ',redo:'Ctrl+Shift+KeyZ',play:'Space',zoom_in:'Equal',zoom_out:'Minus'};
export const playbackAction={id:'play',label:'Воспроизведение / пауза'} as const;
export const movieActions:{id:Exclude<keyof EditorHotkeys,'play'>;label:string}[]=[
  {id:'split',label:'Разделить по курсору'},
  {id:'undo',label:'Отменить изменение'},
  {id:'redo',label:'Повторить изменение'},
  {id:'delete',label:'Удалить фрагмент'},
  {id:'duplicate',label:'Дублировать фрагмент'},
  {id:'export',label:'Экспорт видео'},
  {id:'snapping',label:'Магнитизм'},
  {id:'inspector',label:'Панель фрагмента'},
  {id:'zoom_in',label:'Увеличить таймлайн'},
  {id:'zoom_out',label:'Уменьшить таймлайн'}
];
export const editorActions=[playbackAction,...movieActions];
export function editorHotkeys(settings?:Settings){return {...defaultEditorHotkeys,...settings?.editor_hotkeys};}
export function keyCombination(event:Pick<KeyboardEvent,'code'|'ctrlKey'|'altKey'|'shiftKey'|'metaKey'>){
 if(!event.code||/^(Control|Shift|Alt|Meta)(Left|Right)$/.test(event.code))return '';
 return [event.ctrlKey?'Ctrl':'',event.altKey?'Alt':'',event.shiftKey?'Shift':'',event.metaKey?'Meta':'',event.code].filter(Boolean).join('+');
}
export function hotkeyLabel(value:string){return value.split('+').map(key=>({Space:'Пробел',Equal:'=',Minus:'−',Delete:'Delete',Backspace:'Backspace',ArrowLeft:'←',ArrowRight:'→',Meta:'Win'}[key]??key.replace(/^(Key|Digit)/,''))).join(' + ');}
