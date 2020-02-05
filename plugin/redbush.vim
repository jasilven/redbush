" if exists('g:redbush_loaded') || !has('nvim')
"     finish
" endif


""""""""""""""""""""""
"""" config
""""""""""""""""""""""
let g:redbush_loaded = v:true

fun! s:config(name, default)
    execute "let g:redbush_" . a:name . " = get(g:, 'redbush_" . a:name . "', " . string(a:default) . ")"
endfunction

call s:config('bin','redbush')
call s:config('filepath', '/tmp/redbush-eval.clj')
call s:config('filesize', 1000)
call s:config('is_vertical', 1)
call s:config('winsize', 40)


""""""""""""""""""""""
"""" jobcontrol
""""""""""""""""""""""
if !exists('s:jobid')
    let s:jobid = 0
endif

fun! s:start(...)
    if (s:jobid == 0) || (s:jobid == -1)
        let l:command = [g:redbush_bin, '-f', g:redbush_filepath, '-s', g:redbush_filesize]
        if a:0 == 1
            let l:port = a:1
        elseif (filereadable('.nrepl-port') == 0) && (filereadable('.prepl-port') == 0)
            let l:port = input("Give xREPL port? ")
            if l:port == ''
                echo "No port given, quitting."
                return
            endif 
        endif

        let l:command = extend(l:command, ['-p', l:port ])
        let s:jobid = jobstart(l:command, { 'rpc': v:true , 'on_exit': function('s:exit')})

        if s:jobid == 0
            echoerr "Invalid arguments"
        elseif s:jobid == -1    
            let s:jobid = 0
            echoerr g:redbush_bin . " not executable"
        else 
            exe 'silent! bd! ' . g:redbush_filepath
            exe 'silent! !rm ' . g:redbush_filepath
            call s:logbuf_show(g:redbush_is_vertical, g:redbush_winsize)
        endif
    else
        echo "Already connected:" s:jobid
    endif
endf

fun! s:stop()
    if s:jobid != 0 
        call rpcnotify(s:jobid, 'stop', [])
    endif
endf

fun! s:restart()
    call s:stop()
    sleep 400m
    call s:start()
endf

fun! s:exit(...)
    if len(g:redbush_repl_session_id) == 0
        call s:logbuf_hide()
        echo "Failed to start Redbush"
    endif
    if s:jobid != 0
        let s:jobid = 0
    endif
    let g:redbush_repl_session_id = ''
endf


""""""""""""""""""""""
"""" window stuff 
""""""""""""""""""""""
fun! s:logbuf_show(is_vertical, size)
    let prev_window = winnr()
    if a:is_vertical == 1
        exe 'belowright ' . a:size . 'vsplit' . g:redbush_filepath
    else 
        exe 'belowright ' . a:size . 'split' . g:redbush_filepath
    endif
    exe 'set signcolumn=no'
    exe g:redbush_filesize
    let s:logbuf_bufinfo = getbufinfo(g:redbush_filepath)[0]
    let g:logbuf_winid = bufwinid(g:redbush_filepath)
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
    if bufexists(g:redbush_filepath) 
        let s:logbuf_bufinfo = getbufinfo(g:redbush_filepath)
    endif
endf

fun! s:logbuf_toggle() 
    if bufexists(g:redbush_filepath) 
        let s:logbuf_bufinfo = getbufinfo(g:redbush_filepath)[0]
        if get(s:logbuf_bufinfo, 'hidden') == 1
            call s:logbuf_show(g:redbush_is_vertical,g:redbush_winsize)
        else
            call s:logbuf_hide()
        endif
    endif
endf


""""""""""""""""""""""
"""" nrepl/eval stuff 
""""""""""""""""""""""
if !exists('g:redbush_repl_session_id')
    let g:redbush_repl_session_id = ''
endif

let s:nrepl_defaults = {
        \ "nrepl.middleware.caught/caugh": "clojure.repl/pst",
        \ "nrepl.middleware.caught/print?": 1,
        \ "XXnrepl.middleware.print/print": "n/a",
        \ "XXnrepl.middleware.print/quota": "n/a" }

fun! s:send_to_plugin(event, args)
    let args = extend(a:args, s:nrepl_defaults)
    let args = extend(args, {"session": g:redbush_repl_session_id})
    call rpcnotify(s:jobid, a:event, args)
endf

fun! s:selected_text() 
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
        \ "file": expand("%:p"),
        \ "line": line, 
        \ "column": column,
        \ "code": s:selected_text() }
    call s:send_to_plugin('eval', args)
endf

fun! s:eval_form() 
    normal va(
    normal "_y 
    call s:eval_range()
endf

fun! s:eval_form_time() 
    let [line, column] = getpos("'<")[1:2]
    normal va(
    normal "_y 
    let args = {
        \ "file": expand("%:p"),
        \ "line": line, 
        \ "column": column,
        \ "code": '(time ' . s:selected_text() . " \n)" }
    call s:send_to_plugin('eval', args)
endf

fun! s:eval_file() 
    let lines = ''
    for l in    getline(1, line("$"))
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
        \ "line": 0,
        \ "code": join(lines, "\n"),
        \ "file-path": l:path, 
        \ "file-name": expand("%:t") }

    call s:send_to_plugin('eval', args)
endf

fun! s:run_tests() 
    let args = {
        \ "file": expand("%:p"),
        \ "code": '(clojure.test/run-tests)' }
    call s:send_to_plugin('eval', args)
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
command! RedBushEvalFormTime call s:eval_form_time()

""""""""""""""""""""""
"""" testing
""""""""""""""""""""""
fun! s:interrupt() 
    call s:send_to_plugin('interrupt', {})
endf

command! RedBushInterrupt call s:interrupt()
command! -nargs=1 RedBushConnect call s:start(<q-args>)
