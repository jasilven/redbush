# redbush 
Redbush is a [Neovim] plugin for [Clojure] repls.
It supports both [nrepl] and Clojure's own socket repl (clojure.core.server/io-prepl).
Plugin itself is written in [rust] so it has almost instant startup time provided 
of course that you have nrepl/prepl-server up and running. 
Redbush uses [neovim-lib] for neovim integration.   

Redbush supports very basic set of interactions with the repl (send forms to repl, 
receive and show evaluation results) and should be accompanied with other more specialized 
clojure-related plugins (e.g. for linting, documentation lookup, completion, etc.)  
to have more complete clojure development environment with neovim. 

What makes redbush different from other similar neovim clojure-plugins? 
As redbush is written in rust it has fast and almost instant startup time and it has
effortless/automatic support for both nrepl and prepl (clojure's standard socket repl). 
Those two features were the main goals for redbush from the beginning.

While the basic functionality is considered done, it is still taking baby steps and it's definitely not yet battle-tested thoroughly. 


### Install 
Basic installation requires that you have rust and cargo installed in your system.
Example installation using [vim-plug]: 

```
Plug 'jasilven/redbush', { 'do': 'cargo install --path .' }
```

This fetches, compiles and installs the latest version of redbush binary executable using cargo 
(see https://doc.rust-lang.org/cargo/commands/cargo-install.html for details).
Redbush binary executable is placed according to your cargo settings (typically in `$HOME/.cargo/bin`).


### Configuration

Example configuration with default values (.vimrc/init.vim): 

```
let g:redbush_bin = 'redbush'
let g:redbush_filepath = '/tmp/redbush-eval.clj'
let g:redbush_filesize = 1000 
let g:redbush_is_vertical = v:true
let g:redbush_winsize = 40
```
* `g:redbush_bin` tells where the redbush binary is located. If it's not in your $PATH then full path is required. 
* `g:redbush_filepath` file path/name of the redbush evaluation buffer, that is used to record and show REPL responses. 
* `g:redbush_filesize` redbush evaluation buffer size in lines. 
* `g:redbush_is_vertical` if this is `v:true` then evaluation buffer is shown as vertical split window in neovim otherwise horizontal. 
* `g:redbush_winsize` evaluation buffer window size. For vertical window it's the width and for horizontal window it's the height of the evaluation buffer window. 

You only need to configure those if you are not happy with the defaults. 


### Usage

#### Start nrepl, prepl or both of them
First start your nrepl/prepl server however you wish. 
Here is an example of leiningen project.clj that will start both repls:

```
(defproject myproject "0.1"
  :dependencies [[org.clojure/clojure "1.10.0"]]
  :repl-options {:init (let [port (+ 6000 (rand-int 1000))]
                         (spit ".prepl-port" port)
                         (clojure.core.server/start-server {:accept 'clojure.core.server/io-prepl
                                                            :address "localhost"
                                                            :port port
                                                            :name "prepl"}))})
```

With the above project.clj in place in your project root run:

```
$ lein repl
```
Now you should have both nrepl and prepl available in different ports and the port numbers can be found in `.nrepl-port` and `.prepl-port` accordingly.


### Use the redbush plugin 
There are several neovim commands available that you can use to interact with the plugin and the repl:

#### Starting/Stopping redbush and connecting to the repl
* `RedBushStart` starts redbush plugin which connects to the repl port if there 
is either `.nrepl-port` or `.prepl-port` file containing the repl port number in the current working directory. 
If both of the port-files are missing you should use `RedBushConnect <repl port-number>` to start redbush and connect it to the repl you wish. 
* `RedBushConnect <port>` starts and connects redbush to the repl in port `<port>`. 
With `RedBushConnect` the `.nrepl-port` or '.prepl-port' files are ignored.
* `RedBushRestart` restart redbush.
* `RedBushStop` stop and exit redbush. 

#### Evaluating 
* `RedBushEvalRange` evaluate (visual) range.
* `RedBushEvalForm` evaluate surrounding clojure-form.
* `RedBushEvalFile` evaluate whole file.
* `RedBushEvalFormTime` evaluate surrounding clojure-form with `clojure.core/time`.

#### Show/Hide evaluation buffer 
* `RedBushToggle` toggle evaluation buffer.

#### Running tests 
* `RedBushRunTests` run current namespace tests (using `clojure.test/run-tests`).

All of the above neovim-commands can be mapped as usual to keyboard shortcuts as you like.


[clojure]: https://clojure.org/
[neovim]: https://neovim.io/
[rust]: https://www.rust-lang.org/
[nrepl]: https://nrepl.org/
[vim-plug]: https://github.com/junegunn/vim-plug
[neovim-lib]: https://github.com/daa84/neovim-lib
