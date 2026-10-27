# FTL Randomizer

A randomizer for FTL: Faster Than Light. It randomizes nearly everything about the starting ships: room layout, ship colors, starting systems, crew, weapons, drones and augments.
It runs in a web browser, meaning you don't have to download or execute anything extra, other than maybe a mod manager if you aren't using one, but even that is optional, see below.
TODO screenshots  

## Using the randomizer

Go to TODO URL, then select or drag and drop your `ftl.dat` (default location if you have the game from Steam: `C:\Program Files (x86)\Steam\steamapps\common\FTL Faster Than Light`), then you can save the mod as a `.ftl` file for mod managers.  
I tested it with [Slipstream Mod Manager](https://www.subsetgames.com/forum/viewtopic.php?t=17102) but any of the more recently developed mod managers with Slipstream compatibility should also work.  
Alternatively, you can save it as a patched `ftl.dat` if you don't want to use a mod manager, remember to back up your original `ftl.dat` first.  

## Design decisions

While it may have been interesting to have everything "fully random", I kinda wanted every run to be reasonably likely to be beatable even on Hard difficulty. I've written about it in [documentation/randomizer_ideas.md](documentation/randomizer_ideas.md) and the exact implementation is in [src/randomizer.rs](src/randomizer.rs).  
Essentially, it boils down to making sure each ship starts with a way of reliably taking down the average sector 1-2 ship, be it through weapons, boarding or drones. If the ship starts with only missile weapons, make sure there is a reasonable amount of starting missiles, etc.  
That being said, you are absolutely bound to find ships that are worse than any vanilla ship, and ships that are really overpowered. The high variance is part of the randomizer experience.  

## AI usage disclosure

An AI model, specifically Qwen 3.8 Flash Next running locally on my machine, was used to help me write the code for this program. It did most of the "boring" work so I could focus instead on writing the "interesting" parts, like [src/randomizer.rs](src/randomizer.rs) without having to worry about much of the data unpacking or packing.  
All the gameplay-related decisions have been taken by me. And all the documentation (like this README) has been manually written by me as well.  
I hate AI-generated READMEs because LLMs have no idea what is important enough to be mentioned in a README, and instead dump every single implementation detail that are completely useless for the user to know. I did not want to subject you to this.

## Building the randomizer from source

Requires the usual Rust stuff (Cargo etc.). For the web version, see [web/build.sh](web/build.sh) for more info.
Most notably, you'll need the wasm32 target.  
`rustup target add wasm32-unknown-unknown`  
`cargo install --force wasm-bindgen-cli --version 0.2.100`  
`sh web/build.sh`
You will need to serve the page over http, for instance with `python3 -m http.server 8000 -d .` because loading a .wasm module doesn't work over `file://`.