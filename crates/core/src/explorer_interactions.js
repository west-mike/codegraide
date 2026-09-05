// Shared graph gestures. Drag handlers own movement; activation stays on the node.
const explorerInteractions = (() => {
  let menu=null,pending=null;
  function cancelInspection(){clearTimeout(pending);pending=null}
  function close(){menu?.remove();menu=null}
  function showMenu(event, element, inspect, explore){
    cancelInspection();event.preventDefault();event.stopPropagation();close();
    const box=element.getBoundingClientRect();
    menu=document.createElement('div');menu.className='node-context-menu';menu.setAttribute('role','menu');menu.setAttribute('aria-label','Node actions');
    for(const [label,action] of [['Show details',inspect],['Explore',explore]]){
      const button=document.createElement('button');button.textContent=label;button.setAttribute('role','menuitem');
      button.onclick=()=>{close();action()};menu.append(button);
    }
    document.body.append(menu);
    menu.style.left=Math.max(0,Math.min(event.clientX||box.left,innerWidth-menu.offsetWidth-8))+'px';
    menu.style.top=Math.max(0,Math.min(event.clientY||box.top,innerHeight-menu.offsetHeight-8))+'px';
    menu.firstElementChild.focus();
    menu.onkeydown=e=>{if(e.key==='Escape'){close();element.focus()}else if(['ArrowDown','ArrowUp'].includes(e.key)){e.preventDefault();const buttons=[...menu.children],index=buttons.indexOf(document.activeElement);buttons[(index+1)%buttons.length].focus()}};
  }
  document.addEventListener('pointerdown',e=>{cancelInspection();if(menu&&!menu.contains(e.target))close()});
  window.addEventListener('blur',()=>{cancelInspection();close()});
  function bind(element,{inspect,explore,blocked=()=>false}){
    element.addEventListener('pointerdown',cancelInspection);
    element.addEventListener('click',e=>{e.stopPropagation();cancelInspection();if(!blocked()&&e.detail<2)pending=setTimeout(()=>{pending=null;inspect()},300)});
    element.addEventListener('dblclick',e=>{cancelInspection();e.preventDefault();e.stopPropagation();if(!blocked()){close();explore()}});
    element.addEventListener('contextmenu',e=>showMenu(e,element,inspect,explore));
    element.addEventListener('keydown',e=>{
      cancelInspection();
      if(e.key==='ContextMenu'||e.key==='F10'&&e.shiftKey){showMenu(e,element,inspect,explore);return}
      if(e.key==='Enter'||e.key===' '){e.preventDefault();e.stopPropagation();if(e.key==='Enter'&&e.shiftKey)explore();else inspect()}
    });
  }
  return {bind,close,cancelInspection};
})();
