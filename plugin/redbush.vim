" if exists('g:redbush_loaded') || !has('nvim')
"     finish
" endif
let g:redbush_loaded = v:true

""""""""""""""""""""""
"""" config
""""""""""""""""""""""
fun! s:config(name, default)
  execute "let g:redbush_" . a:name . " = get(g:, 'redbush_" . a:name . "', " . string(a:default) . ")"
endfunction

call s:config('bin','redbush')
call s:config('logfile', '/tmp/redbush-log.clj')
call s:config('logsize', 200)
call s:config('is_vertical', 1)
call s:config('winsize', 40)

""""""""""""""""""""""
"""" jobcontrol
""""""""""""""""""""""
if !exists('s:jobid')
	let s:jobid = 0
endif

fun! s:start()
  if (s:jobid == 0)  || (s:jobid == -1)
    if filereadable('.nrepl-port') == 0
      let l:command = [g:redbush_bin, '-l', g:redbush_logfile, '-s', g:redbush_logsize, '-p', input("Give nREPL port? ")]
    else 
      let l:command = [g:redbush_bin, '-l', g:redbush_logfile, '-s', g:redbush_logsize]
    endif

    let s:jobid = jobstart(l:command, { 'rpc': v:true , 'on_exit': function('s:exit')})

    if s:jobid == 0
      echoerr "invalid arguments"
    elseif s:jobid == -1  
      let s:jobid = 0
      echoerr g:redbush_bin . " not executable"
    else 
      exe 'silent! bd! ' g:redbush_logfile
      call s:logbuf_show(g:redbush_is_vertical, g:redbush_winsize)
    endif
  else
    echo "already connected: " s:jobid
  endif
endf

fun! s:stop()
  if s:jobid != 0 
    call rpcnotify(s:jobid, 'stop', [])
    sleep 400m
    exe 'silent! bd! ' g:redbush_logfile
  endif
	let s:jobid = 0
endf

fun! s:restart()
  call s:stop()
  call s:start()
endf

fun! s:exit(...)
  let s:jobid = 0
  call s:logbuf_hide()
endf


""""""""""""""""""""""
"""" plugin calls 
""""""""""""""""""""""
let s:nrepl_defaults = {
    \ "id": "redbush",
    \ "nrepl.middleware.caught/caugh": "clojure.repl/pst",
    \ "nrepl.middleware.caught/print?": 1,
    \ "XXnrepl.middleware.print/print": "clojure.pprint/pprint",
    \ "XXnrepl.middleware.print/quota": "n/a" }

fun! s:send_to_plugin(event, args)
  call rpcnotify(s:jobid, a:event, extend(a:args, s:nrepl_defaults))
endf

fun! s:seleted_text() 
  let [lnum1, col1] = getpos("'<")[1:2]
  let [lnum2, col2] = getpos("'>")[1:2]
  let lines = getline(lnum1, lnum2)         
  let lines[-1] = lines[-1][: col2 - (&selection == 'inclusive' ? 1 : 2)]
  let lines[0] = lines[0][col1 - 1:]
  return join(lines, "\n")         
endf

fun! s:eval_range() 
  let [line, column] = getpos("'<")[1:2]
  let args = {
    \ "op": "eval", 
    \ "file": expand("%:p"),
    \ "line": line, 
    \ "column": column,
    \ "code": s:seleted_text() }
  call s:send_to_plugin('nrepl', args)
endf

fun! s:eval_form() 
  normal va(
  normal "_y 
  call s:eval_range()
endf

fun! s:eval_file() 
  let lines = ''
  for l in  getline(1, line("$"))
    let l = trim(l)
    let lines = lines . "\n" . l
  endfor

  let splits = split(expand("%:p"), 'src/') 
  if len(splits) == 2
    let l:path = splits[1]
  else 
    let l:path = expand("%:t")
  endif

  let end = getpos("$")[1] 
  let lines = getline(1, end)         
           
  let args = {
    \ "op": "eval", 
    \ "line": 0,
    \ "code": join(lines, "\n"),
    \ "file-path": l:path, 
    \ "file-name": expand("%:t") }

  call s:send_to_plugin('nrepl', args)
endf

fun! s:logbuf_show(is_vertical, size)
	let prev_window = winnr()
  if a:is_vertical == 1
    exe 'belowright ' . a:size . 'vsplit' . g:redbush_logfile
  else 
    exe 'belowright ' . a:size . 'split' . g:redbush_logfile
  endif
  exe g:redbush_logsize
  let s:logbuf_bufinfo = getbufinfo(g:redbush_logfile)[0]
  let g:logbuf_winid = bufwinid(g:redbush_logfile)
	exe prev_window . 'wincmd w'
endf

fun! s:logbuf_hide()
  for winid in get(s:logbuf_bufinfo, 'windows',[])
    let winfo = getwininfo(winid)
    if empty(winfo) == 0
      let winnr = get(winfo[0], 'winnr')
      exe string(winnr) . 'hide'
    endif
  endfor
  if bufexists(g:redbush_logfile) 
    let s:logbuf_bufinfo = getbufinfo(g:redbush_logfile)
  endif
endf

fun! s:logbuf_toggle() 
  if bufexists(g:redbush_logfile) 
    let s:logbuf_bufinfo = getbufinfo(g:redbush_logfile)[0]
    if get(s:logbuf_bufinfo, 'hidden') == 1
      call s:logbuf_show(g:redbush_is_vertical,g:redbush_winsize)
    else
      call s:logbuf_hide()
    endif
  endif
endf

fun! s:run_tests() 
  let args = {
    \ "op": "eval", 
    \ "file": expand("%:p"),
    \ "code": '(clojure.test/run-tests)' }
  call s:send_to_plugin('nrepl', args)
endf

""""""""""""""""""""""
"""" commands
""""""""""""""""""""""
command! RedBushStart call s:start()
command! RedBushStop call s:stop()
command! RedBushRestart call s:restart()
command! -range RedBushEvalRange call s:eval_range()
command! RedBushEvalForm call s:eval_form()
command! RedBushEvalFile call s:eval_file()
command! RedBushToggle call s:logbuf_toggle()
command! RedBushRunTests call s:run_tests()

""""""""""""""""""""""
"""" testing
""""""""""""""""""""""
fun! s:interrupt() 
  let args = {
    \ "op": "interrupt" }
  call s:send_to_plugin('nrepl', args)
endf

command! RedBushInterrupt call s:interrupt()
