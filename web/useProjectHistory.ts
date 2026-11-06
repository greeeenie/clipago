import {useRef,useState} from 'react';
import type {MovieProject} from './api';

export function useProjectHistory(project:MovieProject,setProject:(p:MovieProject)=>void){
 const past=useRef<MovieProject[]>([]);const future=useRef<MovieProject[]>([]);const [,refresh]=useState(0);
 const commit=(next:MovieProject)=>{if(JSON.stringify(next)===JSON.stringify(project))return;past.current=[...past.current.slice(-99),structuredClone(project)];future.current=[];setProject(next);refresh(n=>n+1);};
 const undo=()=>{const previous=past.current.pop();if(!previous)return;future.current.push(structuredClone(project));setProject({...previous,snapping:project.snapping});refresh(n=>n+1);};
 const redo=()=>{const next=future.current.pop();if(!next)return;past.current.push(structuredClone(project));setProject({...next,snapping:project.snapping});refresh(n=>n+1);};
 return {commit,undo,redo,canUndo:past.current.length>0,canRedo:future.current.length>0};
}
