"""Create tiny DOC/DOCX parser fixtures without Office or third-party software.

DOC uses a CFB v3 container and one UTF-16 MS-DOC CLX text piece.
These are parser test fixtures, not user document export functionality.
"""
from pathlib import Path
import struct
import zipfile
from xml.sax.saxutils import escape

root = Path(__file__).resolve().parents[1] / 'output' / 'import-check'
text = (root / 'negative.txt').read_text(encoding='utf-8')
with zipfile.ZipFile(root / 'negative.docx', 'w', zipfile.ZIP_DEFLATED) as archive:
    archive.writestr('[Content_Types].xml', '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>')
    archive.writestr('_rels/.rels', '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>')
    # Mix paragraph boundaries, styled runs and a manual line break.
    lines = text.splitlines()
    paragraphs = ''.join('<w:p><w:r><w:t xml:space="preserve">'+escape(line)+'</w:t></w:r></w:p>' for line in lines[:3])
    paragraphs += '<w:p><w:r><w:t>'+escape(lines[3])+'</w:t><w:br/><w:t>'+escape(lines[4])+'</w:t></w:r></w:p>'
    archive.writestr('word/document.xml', '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>'+paragraphs+'</w:body></w:document>')

END, FREE, FAT = 0xFFFFFFFE, 0xFFFFFFFF, 0xFFFFFFFD
header = bytearray(512)
header[:8] = bytes.fromhex('D0CF11E0A1B11AE1')
def u16(buf, off, value): struct.pack_into('<H', buf, off, value)
def u32(buf, off, value): struct.pack_into('<I', buf, off, value)
u16(header,24,0x3e);u16(header,26,3);u16(header,28,0xfffe);u16(header,30,9);u16(header,32,6)
u32(header,44,1);u32(header,48,16);u32(header,56,4096);u32(header,60,END);u32(header,68,END)
for i in range(109):u32(header,76+4*i,17 if i==0 else FREE)

body = (text.replace('\n','\r')+'\r').encode('utf-16le')
assert len(body)<=2048
word = bytearray(4096)
u16(word,0,0xA5EC);u16(word,2,0x00c1);u16(word,6,0x0409);u16(word,10,4);u16(word,20,0x0409)
u32(word,24,2048);u32(word,28,2048+len(body));u16(word,32,14);u16(word,62,22)
u32(word,76,len(body)//2);u16(word,152,93);u32(word,418,0);u32(word,422,21)
word[2048:2048+len(body)] = body
table=bytearray(4096)
table[0]=2;u32(table,1,16);u32(table,5,0);u32(table,9,len(body)//2);u32(table,15,2048)

directory=bytearray(512)
def entry(index,name,kind,start,size,left=FREE,right=FREE,child=FREE):
    off=index*128;encoded=(name+'\0').encode('utf-16le');directory[off:off+len(encoded)]=encoded
    u16(directory,off+64,len(encoded));directory[off+66]=kind;directory[off+67]=1
    u32(directory,off+68,left);u32(directory,off+72,right);u32(directory,off+76,child)
    u32(directory,off+116,start);struct.pack_into('<Q',directory,off+120,size)
entry(0,'Root Entry',5,END,0,child=1)
entry(1,'WordDocument',2,0,4096,left=2)
entry(2,'0Table',2,8,4096)
fat=bytearray(512)
for i in range(128):u32(fat,4*i,i+1 if i<16 and i not in [7,15] else END if i in [7,15,16] else FAT if i==17 else FREE)
(root/'negative.doc').write_bytes(header+word+table+directory+fat)
print('Created negative.doc and negative.docx')
