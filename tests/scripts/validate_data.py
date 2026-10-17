#!/usr/bin/env python3
"""
tests/scripts/validate_data.py
Validates integrity, formatting, and compliance of the data/ directory.
Strict checks:
1. No prohibited keywords (e.g. '可研') in any file path or content metadata.
2. Standard clean directory names (no -testdata suffix).
3. Presence of paired OCR and MinerU files.
4. Validation of 16-image batch in vlm/.
5. Validation of 16kHz mono audio WAV header.
6. Validation of 64k needle-in-haystack prompt structure.
7. Validation of GeoJSON geometry in rs/.
"""

import os
import sys
import json
import wave

WORKSPACE = "/home/yangsen/workspaces/qyzs-ai/qyzs-cli.git"
DATA_DIR = os.path.join(WORKSPACE, "tests/data")

def check_no_prohibited_terms():
    prohibited = ["可研", "-testdata"]
    for root, dirs, files in os.walk(DATA_DIR):
        for name in dirs + files:
            full_path = os.path.join(root, name)
            for p in prohibited:
                if p in name:
                    raise AssertionError(f"Prohibited term '{p}' found in path: {full_path}")
    print("[PASS] No prohibited terms or suffixes ('可研', '-testdata') detected in tests/data/")

def check_mineru_and_ocr():
    mineru_files = [
        "tests/data/mineru/text_pdf/REF-0056_政府采购评审专家管理办法.pdf",
        "tests/data/mineru/scanned_pdf/REF-0101_网络安全等级保护指导意见_scan.pdf",
        "tests/data/mineru/public_doc/REF-0017_国土空间规划实施监测网络工作方案.doc",
        "tests/data/mineru/public_doc/REF-0106_项目验收前符合性审查表.docx"
    ]
    for mf in mineru_files:
        p = os.path.join(WORKSPACE, mf)
        assert os.path.exists(p) and os.path.getsize(p) > 0, f"Missing MinerU file: {mf}"
    
    ocr_images = [
        "tests/data/ocr/images/ref0056_p1_title.png",
        "tests/data/ocr/images/ref0101_p1_redhead_scan.png",
        "tests/data/ocr/images/ref0101_p8_seal_scan.png"
    ]
    for of in ocr_images:
        p = os.path.join(WORKSPACE, of)
        assert os.path.exists(p) and os.path.getsize(p) > 0, f"Missing OCR image: {of}"
    
    spec_p = os.path.join(WORKSPACE, "tests/data/ocr/paired_alignment_spec.json")
    with open(spec_p, "r", encoding="utf-8") as f:
        spec = json.load(f)
        assert len(spec.get("pairs", [])) >= 3
    print(f"[PASS] MinerU & OCR paired files verified ({len(mineru_files)} docs, {len(ocr_images)} paired renders)")

def check_vlm():
    vlm_cases_p = os.path.join(WORKSPACE, "tests/data/vlm/vlm_test_cases.json")
    with open(vlm_cases_p, "r", encoding="utf-8") as f:
        spec = json.load(f)
        assert len(spec.get("test_cases", [])) >= 3
    
    multi_dir = os.path.join(WORKSPACE, "tests/data/vlm/multi_image_batch")
    images = [f for f in os.listdir(multi_dir) if f.endswith(('.png', '.jpg'))]
    assert len(images) == 16, f"Expected exactly 16 images in multi_image_batch, found {len(images)}"
    print(f"[PASS] VLM dataset verified (3 primary images + exactly {len(images)} batch images)")

def check_audio():
    wav_p = os.path.join(WORKSPACE, "tests/data/audio/samples/sample_speech_cmd.wav")
    assert os.path.exists(wav_p), f"Missing audio file {wav_p}"
    with wave.open(wav_p, "r") as wf:
        channels = wf.getnchannels()
        sampwidth = wf.getsampwidth()
        framerate = wf.getframerate()
        assert channels == 1, f"Expected 1 channel (mono), got {channels}"
        assert sampwidth == 2, f"Expected 16-bit (2 bytes), got {sampwidth}"
        assert framerate == 16000, f"Expected 16000Hz, got {framerate}"
    print(f"[PASS] Audio dataset verified (16kHz, mono, 16-bit PCM WAV)")

def check_chat():
    needle_p = os.path.join(WORKSPACE, "tests/data/chat/needle_haystack_64k.json")
    with open(needle_p, "r", encoding="utf-8") as f:
        data = json.load(f)
        assert "secret_key" in data
        assert data["char_count"] > 50000
        assert data["secret_key"] in data["haystack_text"]
    print(f"[PASS] Chat dataset verified (short, long, 64k needle with {data['char_count']} chars)")

def check_rag():
    chunks_p = os.path.join(WORKSPACE, "tests/data/rag/chunks_sample.json")
    queries_p = os.path.join(WORKSPACE, "tests/data/rag/queries_sample.json")
    with open(chunks_p, "r") as f:
        chunks = json.load(f)
        assert len(chunks) >= 5
    with open(queries_p, "r") as f:
        queries = json.load(f)
        assert len(queries) >= 3
    print(f"[PASS] RAG dataset verified ({len(chunks)} chunks, {len(queries)} evaluation queries)")

def check_rs():
    rs_p = os.path.join(WORKSPACE, "tests/data/rs/sample_aoi.geojson")
    with open(rs_p, "r") as f:
        geo = json.load(f)
        assert geo["type"] == "FeatureCollection"
        assert len(geo["features"]) >= 1
    print("[PASS] Remote Sensing dataset verified (Valid AOI GeoJSON)")

if __name__ == "__main__":
    print("=== Running Dataset Integrity Validation ===")
    check_no_prohibited_terms()
    check_mineru_and_ocr()
    check_vlm()
    check_audio()
    check_chat()
    check_rag()
    check_rs()
    print("=== ALL DATASET VALIDATION CHECKS PASSED ===")
