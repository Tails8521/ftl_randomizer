use image::Rgba;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::fs::{create_dir_all, File};
use std::io::{self, BufRead, BufReader};
use std::path::Path;

#[derive(Debug)]
struct Room {
    id: i32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

#[derive(Debug)]
struct Door {
    x: i32,
    y: i32,
    room_a: i32,
    room_b: i32,
    is_vertical: bool,
}

struct ShipLayout {
    horizontal: i32,
    vertical: i32,
    rooms: Vec<Room>,
    doors: Vec<Door>,
}

fn parse_layout(path: &Path) -> io::Result<ShipLayout> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    let mut horizontal = 0;
    let mut vertical = 0;
    let mut rooms = Vec::new();
    let mut doors = Vec::new();

    while let Some(Ok(line)) = lines.next() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        match line {
            "HORIZONTAL" => {
                if let Some(Ok(l)) = lines.next() {
                    horizontal = l.trim().parse().unwrap_or(0);
                }
            }
            "VERTICAL" => {
                if let Some(Ok(l)) = lines.next() {
                    vertical = l.trim().parse().unwrap_or(0);
                }
            }
            "ROOM" => {
                let mut values = Vec::new();
                while values.len() < 5 {
                    if let Some(Ok(l)) = lines.next() {
                        values.push(l.trim().parse::<i32>().unwrap_or(0));
                    } else {
                        break;
                    }
                }
                if values.len() == 5 {
                    rooms.push(Room {
                        id: values[0],
                        x: values[1],
                        y: values[2],
                        w: values[3],
                        h: values[4],
                    });
                }
            }
            "DOOR" => {
                let mut values = Vec::new();
                while values.len() < 5 {
                    if let Some(Ok(l)) = lines.next() {
                        values.push(l.trim().parse::<i32>().unwrap_or(0));
                    } else {
                        break;
                    }
                }
                if values.len() == 5 {
                    doors.push(Door {
                        x: values[0],
                        y: values[1],
                        room_a: values[2],
                        room_b: values[3],
                        is_vertical: values[4] == 1,
                    });
                }
            }
            _ => {}
        }
    }

    Ok(ShipLayout {
        horizontal,
        vertical,
        rooms,
        doors,
    })
}

fn parse_offsets(path: &Path) -> (i32, i32, i32, i32) {
    let mut reader = Reader::from_file(path).expect("Failed to open XML file");

    let mut img_x = 0;
    let mut img_y = 0;
    let mut floor_x = 0;
    let mut floor_y = 0;

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match e.name().as_ref() {
                b"img" => {
                    for attr in e.attributes().flatten() {
                        match attr.key.as_ref() {
                            b"x" => {
                                img_x = std::str::from_utf8(&attr.value)
                                    .unwrap_or_default()
                                    .parse()
                                    .unwrap_or(0)
                            }
                            b"y" => {
                                img_y = std::str::from_utf8(&attr.value)
                                    .unwrap_or_default()
                                    .parse()
                                    .unwrap_or(0)
                            }
                            _ => {}
                        }
                    }
                }
                b"floor" => {
                    for attr in e.attributes().flatten() {
                        match attr.key.as_ref() {
                            b"x" => {
                                floor_x = std::str::from_utf8(&attr.value)
                                    .unwrap_or_default()
                                    .parse()
                                    .unwrap_or(0)
                            }
                            b"y" => {
                                floor_y = std::str::from_utf8(&attr.value)
                                    .unwrap_or_default()
                                    .parse()
                                    .unwrap_or(0)
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => panic!("Error parsing XML: {}", e),
            _ => {}
        }
        buf.clear();
    }

    (img_x, img_y, floor_x, floor_y)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ship_name = "energy_cruiser";
    let layout_str = format!("extracted/data/{}.txt", ship_name);
    let image_str = format!("extracted/img/ship/{}_base.png", ship_name);
    let floor_str = format!("extracted/img/ship/{}_floor.png", ship_name);
    let xml_str = format!("extracted/data/{}.xml", ship_name);
    let layout_path = Path::new(&layout_str);
    let image_path = Path::new(&image_str);
    let floor_path = Path::new(&floor_str);
    let xml_path = Path::new(&xml_str);

    println!("Loading layout from {:?}", layout_path);
    let layout = parse_layout(layout_path)?;
    println!("Loading image from {:?}", image_path);
    let mut img = image::open(image_path)?.to_rgba8();

    let (img_x, img_y, floor_x, floor_y) = parse_offsets(xml_path);
    println!(
        "Offsets: img(x={}, y={}), floor(x={}, y={})",
        img_x, img_y, floor_x, floor_y
    );

    if floor_path.exists() {
        println!("Overlaying floor from {:?}", floor_path);
        let floor_img = image::open(floor_path)?.to_rgba8();
        let (fw, fh) = floor_img.dimensions();

        for y in 0..fh {
            for x in 0..fw {
                let px = floor_img.get_pixel(x, y);
                if px[3] > 0 {
                    let target_x = x as i32 + floor_x;
                    let target_y = y as i32 + floor_y;
                    if target_x >= 0
                        && target_x < img.width() as i32
                        && target_y >= 0
                        && target_y < img.height() as i32
                    {
                        img.put_pixel(target_x as u32, target_y as u32, *px);
                    }
                }
            }
        }
    }

    let (width, height) = img.dimensions();
    println!("Image dimensions: {}x{}", width, height);

    let tile_size = 35;
    let start_x = -img_x + layout.horizontal;
    let start_y = -img_y + layout.vertical;

    println!("Calculated start coordinates: x={}, y={}", start_x, start_y);

    for room in &layout.rooms {
        let rx = start_x + room.x * tile_size;
        let ry = start_y + room.y * tile_size;
        let rw = room.w * tile_size;
        let rh = room.h * tile_size;

        println!(
            "Overlaying room {}: x={}, y={}, w={}, h={}",
            room.id, rx, ry, rw, rh
        );

        for x in rx..(rx + rw) {
            for y in ry..(ry + rh) {
                if x >= 0 && x < width as i32 && y >= 0 && y < height as i32 {
                    let is_border = x == rx || x == rx + rw - 1 || y == ry || y == ry + rh - 1;
                    let color = if is_border {
                        Rgba([0, 255, 0, 255])
                    } else {
                        Rgba([255, 0, 255, 255])
                    };
                    img.put_pixel(x as u32, y as u32, color);
                }
            }
        }
    }

    for door in &layout.doors {
        let dx = start_x + door.x * tile_size;
        let dy = start_y + door.y * tile_size;

        if door.is_vertical {
            for y in dy..(dy + tile_size) {
                for ox in -1..1 {
                    let x = dx + ox;
                    if x >= 0 && x < width as i32 && y >= 0 && y < height as i32 {
                        img.put_pixel(x as u32, y as u32, Rgba([0, 0, 255, 255]));
                    }
                }
            }
        } else {
            for x in dx..(dx + tile_size) {
                for oy in -1..1 {
                    let y = dy + oy;
                    if x >= 0 && x < width as i32 && y >= 0 && y < height as i32 {
                        img.put_pixel(x as u32, y as u32, Rgba([0, 0, 255, 255]));
                    }
                }
            }
        }
    }

    let output_dir = "temp";
    create_dir_all(output_dir)?;
    let output_path = Path::new(output_dir).join("ship_visualized.png");
    img.save(&output_path)?;
    println!("Saved visualized ship to {}", output_path.display());

    Ok(())
}
